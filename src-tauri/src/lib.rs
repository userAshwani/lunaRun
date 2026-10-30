//! Thin Tauri adapter: maps commands and events to `luna-core`. No business logic here.

mod commands;
pub mod events;

use luna_core::LunaHome;
use luna_core::jobs::{JobManager, JobOptions};
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
            commands::job_cancel
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
