//! Thin Tauri adapter: maps commands and events to `luna-core`. No business logic here.

#[allow(
    clippy::expect_used,
    reason = "no UI exists to report to if the shell fails to start"
)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
