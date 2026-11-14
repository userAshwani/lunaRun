//! Tauri commands. Each one only maps arguments to `luna-core` and returns its result.
//! Every command returns `Result<T, LunaError>` (docs/ARCHITECTURE.md §6).
//!
//! Adding a command: write it here, add it to `COMMANDS` in `build.rs` and to `generate_handler!`
//! in `lib.rs`, then grant `allow-<command-name>` in `capabilities/default.json`.

use luna_core::diagnostics::{self, AppInfo, Arch, Os};
use luna_core::jobs::{JobInfo, JobManager, StartedJob};
use luna_core::projects::{
    DetectionData, Project, ProjectDetection, ProjectPatch, ProjectStore, Surroundings,
};
use luna_core::registry::{CalDate, RegistryHolder, RegistryStatus, RuntimesAvailable};
use luna_core::runtimes::{
    InstalledView, Installer, UninstallOptions, Uninstaller, UsageReport, list_installed,
};
use luna_core::services::{
    Leftover, PortAllocator, ServiceAction, ServiceController, ServiceLogs, ServiceRow, Supervisor,
    list_services,
};
use luna_core::{LunaError, LunaHome};
use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::registry;
use crate::services::Leftovers;

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

/// The services lunaRun can run from what is installed, with their state, ports and what the first start will do. Reads
/// only: it writes no config and takes no port (D-031). Live changes arrive as `service:state` events.
#[tauri::command]
pub fn services_list(
    home: State<'_, LunaHome>,
    ports: State<'_, PortAllocator>,
    supervisor: State<'_, Supervisor>,
) -> Result<Vec<ServiceRow>, LunaError> {
    let installed = list_installed(&home)?;
    list_services(&ports, &supervisor, &home, &installed, Os::current())
}

/// Services found still running from an earlier session when the app started (found once, at startup).
#[tauri::command]
pub fn services_leftovers(leftovers: State<'_, Leftovers>) -> Result<Vec<Leftover>, LunaError> {
    Ok(leftovers.list())
}

/// Start a service (a row of the Services page) as a background job named `service:<id>`: `job:progress` (phase
/// `setup` for a database's first start, then `starting`) and `job:done`. The job succeeds when the service **runs**.
/// Refused with `BUSY` while the app quits. Asking again while it starts returns that job.
#[tauri::command]
pub fn service_start(
    id: String,
    controller: State<'_, ServiceController>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    controller.run(&jobs, &id, ServiceAction::Start)
}

/// Stop a service (every worker of a PHP pool) as a background job. Stopping what is not running is a success.
#[tauri::command]
pub fn service_stop(
    id: String,
    controller: State<'_, ServiceController>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    controller.run(&jobs, &id, ServiceAction::Stop)
}

/// Stop a service, then start it again, as one background job. Refused with `BUSY` while the app quits.
#[tauri::command]
pub fn service_restart(
    id: String,
    controller: State<'_, ServiceController>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    controller.run(&jobs, &id, ServiceAction::Restart)
}

/// "Use port N instead": give a stopped service other ports than the ones that turned out to be taken, and start it.
#[tauri::command]
pub fn service_port_reassign(
    id: String,
    controller: State<'_, ServiceController>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    controller.reassign_and_start(&jobs, &id)
}

/// What a row's services have printed: for each service its log sources (`output` first), each with its last
/// `tail_lines` lines (at most 2,000), oldest first. New lines arrive as `service:log` events.
#[tauri::command]
pub fn service_logs(
    id: String,
    tail_lines: usize,
    controller: State<'_, ServiceController>,
) -> Result<Vec<ServiceLogs>, LunaError> {
    controller.logs(&id, tail_lines)
}

/// Take over a service an earlier session of the app left running. `NOT_FOUND` when nothing like it runs or its
/// version is no longer installed, `INVALID_PATH` when what runs is another program.
#[tauri::command]
pub async fn service_adopt(
    id: String,
    controller: State<'_, ServiceController>,
    leftovers: State<'_, Leftovers>,
) -> Result<(), LunaError> {
    controller.adopt(&id).await?;
    leftovers.remove(&id);
    Ok(())
}

/// End a service an earlier session of the app left running (asked to leave, ended after ten seconds).
#[tauri::command]
pub async fn service_stop_leftover(
    id: String,
    controller: State<'_, ServiceController>,
    leftovers: State<'_, Leftovers>,
) -> Result<(), LunaError> {
    controller.stop_leftover(&id).await?;
    leftovers.remove(&id);
    Ok(())
}

/// Every project, in the order they were added. Reads the stored list only: no project folder is opened.
#[tauri::command]
pub fn projects_list(projects: State<'_, ProjectStore>) -> Result<Vec<Project>, LunaError> {
    Ok(projects.list())
}

#[tauri::command]
pub fn project_get(id: String, projects: State<'_, ProjectStore>) -> Result<Project, LunaError> {
    projects.get(&id)
}

/// Add a folder as a project (`INVALID_PATH` when it is not an absolute folder that exists, `PROJECT_EXISTS` when it is
/// already one), read it, and say what it needs and what to run it with. Reads the folder; writes nothing into it.
#[tauri::command]
pub async fn project_add(
    path: String,
    projects: State<'_, ProjectStore>,
    data: State<'_, Arc<DetectionData>>,
    home: State<'_, LunaHome>,
    registry: State<'_, RegistryHolder>,
) -> Result<ProjectDetection, LunaError> {
    let (store, data) = (projects.inner().clone(), data.inner().clone());
    let (installed, available) = world(&home, &registry)?;
    blocking(move || {
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: Os::current(),
        };
        store.add(std::path::Path::new(&path), &data, &around)
    })
    .await
}

/// Read the project's folder again ("Re-scan") and say what it needs now and what to run it with.
#[tauri::command]
pub async fn project_detect(
    id: String,
    projects: State<'_, ProjectStore>,
    data: State<'_, Arc<DetectionData>>,
    home: State<'_, LunaHome>,
    registry: State<'_, RegistryHolder>,
) -> Result<ProjectDetection, LunaError> {
    let (store, data) = (projects.inner().clone(), data.inner().clone());
    let (installed, available) = world(&home, &registry)?;
    blocking(move || {
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: Os::current(),
        };
        store.inspect(&id, &data, &around)
    })
    .await
}

/// Rename a project, change its document root, or choose its PHP or Node by hand (or back to automatic). A part that
/// is not acceptable is `INVALID_INPUT` (`details.field` names it) and nothing is changed.
#[tauri::command]
pub async fn project_update(
    id: String,
    patch: ProjectPatch,
    projects: State<'_, ProjectStore>,
    data: State<'_, Arc<DetectionData>>,
) -> Result<Project, LunaError> {
    let (store, data) = (projects.inner().clone(), data.inner().clone());
    blocking(move || store.update(&id, &patch, &data)).await
}

/// Forget a project. The folder and its files are never touched.
#[tauri::command]
pub fn project_remove(id: String, projects: State<'_, ProjectStore>) -> Result<(), LunaError> {
    projects.remove(&id)
}

/// What is installed and what the registry offers for this computer: what a project is resolved against.
fn world(
    home: &LunaHome,
    registry: &RegistryHolder,
) -> Result<
    (
        Vec<luna_core::runtimes::InstalledRuntime>,
        Vec<luna_core::registry::RuntimeCatalog>,
    ),
    LunaError,
> {
    let today = CalDate::today()
        .ok_or_else(|| LunaError::internal("the system clock does not give a usable date"))?;
    Ok((
        list_installed(home)?,
        registry
            .available(Os::current(), Arch::current(), today)
            .runtimes,
    ))
}

/// Run work that reads a project's folder off the thread that serves the window.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, LunaError> + Send + 'static,
) -> Result<T, LunaError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| LunaError::internal(format!("a background task failed: {err}")))?
}
