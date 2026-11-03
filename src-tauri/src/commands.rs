//! Tauri commands. Each one only maps arguments to `luna-core` and returns its result.
//! Every command returns `Result<T, LunaError>` (docs/ARCHITECTURE.md §6).
//!
//! Adding a command: write it here, add it to `COMMANDS` in `build.rs` and to `generate_handler!`
//! in `lib.rs`, then grant `allow-<command-name>` in `capabilities/default.json`.

use luna_core::diagnostics::{self, AppInfo, Arch, Os};
use luna_core::jobs::{JobInfo, JobManager};
use luna_core::registry::{CalDate, RegistryHolder, RegistryStatus, RuntimesAvailable};
use luna_core::runtimes::{InstalledView, list_installed};
use luna_core::{LunaError, LunaHome};
use tauri::{AppHandle, State};

use crate::registry;

#[tauri::command]
pub fn app_info(home: State<'_, LunaHome>) -> Result<AppInfo, LunaError> {
    Ok(diagnostics::app_info(env!("CARGO_PKG_VERSION"), &home))
}

/// Background jobs, running ones first, then the newest finished ones.
#[tauri::command]
pub fn jobs_list(jobs: State<'_, JobManager>) -> Result<Vec<JobInfo>, LunaError> {
    Ok(jobs.list())
}

/// `Ok(true)`: cancellation requested. `Ok(false)`: the job had already finished.
#[tauri::command]
pub fn job_cancel(job_id: String, jobs: State<'_, JobManager>) -> Result<bool, LunaError> {
    jobs.cancel(&job_id)
}

/// Every runtime with the versions that exist for this computer, and where the list came from.
#[tauri::command]
pub fn runtimes_available(
    registry: State<'_, RegistryHolder>,
) -> Result<RuntimesAvailable, LunaError> {
    let today = CalDate::today()
        .ok_or_else(|| LunaError::internal("the system clock does not give a usable date"))?;
    Ok(registry.available(Os::current(), Arch::current(), today))
}

/// The installed runtimes, by name and newest version first.
#[tauri::command]
pub fn runtimes_installed(home: State<'_, LunaHome>) -> Result<Vec<InstalledView>, LunaError> {
    Ok(list_installed(&home)?
        .iter()
        .map(InstalledView::from)
        .collect())
}

/// Look for a fresher version list now (network, saved copy, built-in copy) and use the result.
#[tauri::command]
pub async fn registry_refresh(
    app: AppHandle,
    registry: State<'_, RegistryHolder>,
    home: State<'_, LunaHome>,
) -> Result<RegistryStatus, LunaError> {
    registry::refresh_and_announce(&app, &registry, &home).await
}
