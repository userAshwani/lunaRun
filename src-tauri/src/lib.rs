//! Thin Tauri adapter: maps commands and events to `luna-core`. No business logic here.

pub mod commands;
pub mod events;
mod registry;
pub mod services;
pub mod tray;

use std::sync::Arc;

use luna_core::LunaHome;
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions};
use luna_core::projects::{DetectionData, ProjectStore};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::{Installer, cleanup_incomplete};
use tauri::Manager;

#[allow(
    clippy::expect_used,
    reason = "no UI exists to report to if the shell fails to start"
)]
pub fn run() {
    let app = tauri::Builder::default()
        // The native folder picker of the Projects page (and nothing else: only `dialog:allow-open` is granted).
        .plugin(tauri_plugin_dialog::init())
        // Launch at login (FR-AP-03, D-045): started with `--hidden`, lunaRun opens in the tray, not as a window. Used from
        // Rust only (`launch_at_login_*`): the page gets no permission of this plugin.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![tray::HIDDEN_ARG]),
        ))
        .setup(|app| {
            let home = LunaHome::resolve()?;
            home.ensure_layout()?;
            // Remove what an install or uninstall that died halfway left behind, before anything new starts.
            // Failing to clean up must not stop the app from opening.
            let _ = cleanup_incomplete(&home);

            // The page can list runtimes at once from the copy built into the app, while the fresh list
            // loads in the background and is announced with the `registry:updated` event.
            let registry = RegistryHolder::with_bundled()?;
            app.manage(registry.clone());
            let site_registry = registry.clone();
            let (handle, refresh_home) = (app.handle().clone(), home.clone());
            tauri::async_runtime::spawn(async move {
                let _ = registry::refresh_and_announce(&handle, &registry, &refresh_home).await;
            });
            app.manage(Installer::new(
                home.clone(),
                Downloader::new(DownloadOptions::default())?,
            ));
            // The supervisor, the port allocator, services left from an earlier session (looked for once, here), the
            // quit state, and the uninstaller, which asks the supervisor what runs from a version before it removes it.
            let projects = ProjectStore::open(&home)?;
            let data = Arc::new(DetectionData::bundled()?);
            services::manage(app.handle(), &home, &projects, &data, &site_registry);
            app.manage(projects);
            app.manage(data);
            app.manage(home);

            // Jobs run on Tauri's own Tokio runtime and report to the webview through events.
            let runtime = tauri::async_runtime::handle().inner().clone();
            let sink = events::TauriJobSink::new(app.handle().clone());
            app.manage(JobManager::new(runtime, sink, JobOptions::default()));

            // The tray icon (FR-AP-01). Without one (a desktop with no tray), closing the window quits, as before.
            let has_tray = tray::create(app.handle());
            // The window starts hidden (tauri.conf.json) and is shown here, unless lunaRun was started at login with
            // `--hidden` and there is a tray to open it from (D-045): then it waits in the tray.
            let started_hidden = std::env::args().any(|arg| arg == tray::HIDDEN_ARG);
            if !(started_hidden && has_tray) {
                tray::show_window(app.handle());
            }
            Ok(())
        })
        // Closing the window hides it to the tray and the services keep running (FR-AP-01, D-043); the tray's Quit
        // stops them. With no tray, closing quits: while a service runs the window stays open to show the orderly
        // stop (D-029, D-033).
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            let app = window.app_handle();
            if tray::is_present(app) && !app.state::<services::QuitState>().is_quitting() {
                api.prevent_close();
                let _ = window.hide();
                return;
            }
            if let services::CloseDecision::Hold { start_quit } = services::on_close_requested(app)
            {
                api.prevent_close();
                if start_quit {
                    services::spawn_quit(app, 0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::jobs_list,
            commands::job_cancel,
            commands::runtimes_available,
            commands::runtimes_installed,
            commands::registry_refresh,
            commands::runtime_install,
            commands::runtime_uninstall,
            commands::runtime_usage,
            commands::services_list,
            commands::services_leftovers,
            commands::service_start,
            commands::service_stop,
            commands::service_restart,
            commands::service_port_reassign,
            commands::service_logs,
            commands::service_adopt,
            commands::service_stop_leftover,
            commands::projects_list,
            commands::project_get,
            commands::project_add,
            commands::project_detect,
            commands::project_update,
            commands::project_remove,
            commands::sites_list,
            commands::sites_sync,
            commands::project_start,
            commands::project_stop,
            commands::project_open,
            commands::project_database,
            commands::project_env_port_preview,
            commands::project_env_port_apply,
            commands::project_database_status,
            commands::project_database_create,
            commands::onboarding_get,
            commands::onboarding_complete,
            commands::onboarding_check_home,
            commands::onboarding_choose_home,
            commands::app_restart,
            commands::settings_get,
            commands::settings_set,
            commands::launch_at_login_get,
            commands::launch_at_login_set
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Closing the last window asks to exit. Before it does, every service is stopped in order (web, PHP, databases):
    // the first request starts that and makes Tauri wait, and the exit it asks for when done goes through (D-029).
    app.run(|handle, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event
            && !services::on_exit_requested(handle, code)
        {
            api.prevent_exit();
        }
    });
}
