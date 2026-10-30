//! Thin Tauri adapter: maps commands and events to `luna-core`. No business logic here.

mod commands;

use luna_core::LunaHome;
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
