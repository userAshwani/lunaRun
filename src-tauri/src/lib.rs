//! Thin Tauri adapter: maps commands and events to `luna-core`. No business logic here.

pub mod commands;
pub mod events;
mod registry;

use luna_core::LunaHome;
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::{Installer, Uninstaller, cleanup_incomplete};
use tauri::Manager;

#[allow(
    clippy::expect_used,
    reason = "no UI exists to report to if the shell fails to start"
)]
pub fn run() {
    tauri::Builder::default()
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
            let (handle, refresh_home) = (app.handle().clone(), home.clone());
            tauri::async_runtime::spawn(async move {
                let _ = registry::refresh_and_announce(&handle, &registry, &refresh_home).await;
            });
            // Installing and uninstalling share the job manager below. Nothing reports a version as in use
            // until the supervisor (M1.4) and the project store (M1.7) can say so.
            app.manage(Installer::new(
                home.clone(),
                Downloader::new(DownloadOptions::default())?,
            ));
            app.manage(Uninstaller::new(home.clone()));
            app.manage(home);

            // Jobs run on Tauri's own Tokio runtime and report to the webview through events.
            let runtime = tauri::async_runtime::handle().inner().clone();
            let sink = events::TauriJobSink::new(app.handle().clone());
            app.manage(JobManager::new(runtime, sink, JobOptions::default()));
            Ok(())
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
            commands::runtime_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
