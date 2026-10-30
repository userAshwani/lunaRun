//! Tauri commands. Each one only maps arguments to `luna-core` and returns its result.
//! Every command returns `Result<T, LunaError>` (docs/ARCHITECTURE.md §6).
//!
//! Adding a command: write it here, add it to `COMMANDS` in `build.rs` and to `generate_handler!`
//! in `lib.rs`, then grant `allow-<command-name>` in `capabilities/default.json`.

use luna_core::diagnostics::{self, AppInfo};
use luna_core::{LunaError, LunaHome};
use tauri::State;

#[tauri::command]
pub fn app_info(home: State<'_, LunaHome>) -> Result<AppInfo, LunaError> {
    Ok(diagnostics::app_info(env!("CARGO_PKG_VERSION"), &home))
}
