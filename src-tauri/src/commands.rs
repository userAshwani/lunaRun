//! Tauri commands. Each one only maps arguments to `luna-core` and returns its result.
//! Every command returns `Result<T, LunaError>` (docs/ARCHITECTURE.md §6).
//!
//! Adding a command: write it here, add it to `COMMANDS` in `build.rs` and to `generate_handler!`
//! in `lib.rs`, then grant `allow-<command-name>` in `capabilities/default.json`.

use luna_core::diagnostics::{self, AppInfo, Arch, Os};
use luna_core::jobs::{JobInfo, JobManager, StartedJob};
use luna_core::registry::{CalDate, RegistryHolder, RegistryStatus, RuntimesAvailable};
use luna_core::runtimes::{
    InstalledView, Installer, UninstallOptions, Uninstaller, UsageReport, list_installed,
};
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

/// Install `runtime` `version` as a background job (`job:progress` / `job:done` events). Asking again for a
/// version that is being installed returns that job; while it is being uninstalled the answer is `BUSY`.
#[tauri::command]
pub fn runtime_install(
    runtime: String,
    version: String,
    registry: State<'_, RegistryHolder>,
    installer: State<'_, Installer>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    installer.start(&jobs, registry.registry(), &runtime, &version)
}

/// Uninstall `runtime` `version` as a background job. `delete_data` also deletes the databases of that version
/// (the user asked for it in the dialog); `confirm_bound_projects` says the user saw the projects that use it.
#[tauri::command]
pub fn runtime_uninstall(
    runtime: String,
    version: String,
    delete_data: bool,
    confirm_bound_projects: bool,
    uninstaller: State<'_, Uninstaller>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    let options = UninstallOptions {
        delete_data,
        confirm_bound_projects,
    };
    uninstaller.start(&jobs, &runtime, &version, options)
}

/// What the uninstall dialog shows: who uses this version and where its databases are. Read-only.
#[tauri::command]
pub fn runtime_usage(
    runtime: String,
    version: String,
    uninstaller: State<'_, Uninstaller>,
) -> Result<UsageReport, LunaError> {
    uninstaller.usage_report(&runtime, &version)
}
