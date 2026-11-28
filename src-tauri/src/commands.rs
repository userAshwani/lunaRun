//! Tauri commands. Each one only maps arguments to `luna-core` and returns its result.
//! Every command returns `Result<T, LunaError>` (docs/ARCHITECTURE.md §6).
//!
//! Adding a command: write it here, add it to `COMMANDS` in `build.rs` and to `generate_handler!`
//! in `lib.rs`, then grant `allow-<command-name>` in `capabilities/default.json`.

use luna_core::dbadmin::{DatabaseCreated, DatabaseStatus};
use luna_core::diagnostics::{self, AppInfo, Arch, Os};
use luna_core::jobs::{JobInfo, JobManager, StartedJob};
use luna_core::launch::{DatabaseSettings, ProjectAction, ProjectLauncher};
use luna_core::onboarding::{self, HomeChoice, OnboardingView};
use luna_core::opener::{self, OpenTarget};
use luna_core::projects::envfix::{EnvFixApplied, PortChange};
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
use luna_core::settings::{self, Settings, SettingsView};
use luna_core::sites::{SiteManager, SitesView};
use luna_core::terminal;
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
    sites: State<'_, SiteManager>,
) -> Result<ProjectDetection, LunaError> {
    let (store, data) = (projects.inner().clone(), data.inner().clone());
    let (installed, available) = world(&home, &registry)?;
    let defaults = settings::load(&home)?.runtime_defaults();
    let added = blocking(move || {
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: Os::current(),
            defaults: &defaults,
        };
        store.add(std::path::Path::new(&path), &data, &around)
    })
    .await?;
    apply_sites(&sites).await;
    Ok(added)
}

/// Read the project's folder again ("Re-scan") and say what it needs now and what to run it with.
#[tauri::command]
pub async fn project_detect(
    id: String,
    projects: State<'_, ProjectStore>,
    data: State<'_, Arc<DetectionData>>,
    home: State<'_, LunaHome>,
    registry: State<'_, RegistryHolder>,
    sites: State<'_, SiteManager>,
) -> Result<ProjectDetection, LunaError> {
    let (store, data) = (projects.inner().clone(), data.inner().clone());
    let (installed, available) = world(&home, &registry)?;
    let defaults = settings::load(&home)?.runtime_defaults();
    let scanned = blocking(move || {
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: Os::current(),
            defaults: &defaults,
        };
        store.inspect(&id, &data, &around)
    })
    .await?;
    // A rescan can change the document root a project is served from.
    apply_sites(&sites).await;
    Ok(scanned)
}

/// Rename a project, change its document root, or choose its PHP or Node by hand (or back to automatic). A part that
/// is not acceptable is `INVALID_INPUT` (`details.field` names it) and nothing is changed.
#[tauri::command]
pub async fn project_update(
    id: String,
    patch: ProjectPatch,
    projects: State<'_, ProjectStore>,
    data: State<'_, Arc<DetectionData>>,
    sites: State<'_, SiteManager>,
) -> Result<Project, LunaError> {
    let (store, data) = (projects.inner().clone(), data.inner().clone());
    let updated = blocking(move || store.update(&id, &patch, &data)).await?;
    apply_sites(&sites).await;
    Ok(updated)
}

/// Forget a project. The folder and its files are never touched.
#[tauri::command]
pub async fn project_remove(
    id: String,
    projects: State<'_, ProjectStore>,
    sites: State<'_, SiteManager>,
) -> Result<(), LunaError> {
    projects.remove(&id)?;
    // It stops being served, and its port is free again.
    apply_sites(&sites).await;
    Ok(())
}

/// The site of every project, whether there is a web server to serve it, and why the last change could not be given to
/// Caddy, if it could not. Reads the project folders; giving a new site its port is the only write.
#[tauri::command]
pub async fn sites_list(sites: State<'_, SiteManager>) -> Result<SitesView, LunaError> {
    let sites = sites.inner().clone();
    blocking(move || sites.view()).await
}

/// Give Caddy the sites as they are now (write the Caddyfile and, when Caddy runs, validate and reload it) and say how
/// that went: the "Try again" of the project page.
#[tauri::command]
pub async fn sites_sync(sites: State<'_, SiteManager>) -> Result<SitesView, LunaError> {
    let sites = sites.inner().clone();
    apply_sites(&sites).await;
    blocking(move || sites.view()).await
}

/// Start a project as a background job `project:<id>` (`job:progress`, `job:done`, Cancel): its database (when its `.env`
/// expects MySQL or MariaDB and that is installed), its PHP pool and Caddy with the project in the Caddyfile, each only
/// when it is not already running. All or none. `NO_SITE` when its PHP is not installed or its folder is gone.
#[tauri::command]
pub fn project_start(
    id: String,
    launcher: State<'_, ProjectLauncher>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    launcher.run(&jobs, &id, ProjectAction::Start)
}

/// Stop a project as a background job: Caddy stops serving it, and the services its start began are stopped unless
/// another started project needs them. What the user started by hand is left running.
#[tauri::command]
pub fn project_stop(
    id: String,
    launcher: State<'_, ProjectLauncher>,
    jobs: State<'_, JobManager>,
) -> Result<StartedJob, LunaError> {
    launcher.run(&jobs, &id, ProjectAction::Stop)
}

/// Open the site of a project in the browser, its folder in the file manager or in Visual Studio Code, or a terminal in
/// its folder with its own PHP, Composer and Node first on `PATH`. The page only names the project and what to open: the
/// address and the folder are the ones lunaRun stored.
#[tauri::command]
pub async fn project_open(
    id: String,
    target: OpenTarget,
    home: State<'_, LunaHome>,
    projects: State<'_, ProjectStore>,
    sites: State<'_, SiteManager>,
) -> Result<(), LunaError> {
    let (home, projects, sites) = (
        home.inner().clone(),
        projects.inner().clone(),
        sites.inner().clone(),
    );
    blocking(move || open_project(&home, &projects, &sites, &id, target)).await
}

/// What `project_open` does, for the tray too: the address and the folder are the ones lunaRun stored, and the only things
/// asked for are a project id and what to open.
///
/// # Errors
/// As `project_open`.
pub fn open_project(
    home: &LunaHome,
    projects: &ProjectStore,
    sites: &SiteManager,
    id: &str,
    target: OpenTarget,
) -> Result<(), LunaError> {
    if target == OpenTarget::Terminal {
        return terminal::open(home, Os::current(), projects, sites, id);
    }
    let project = projects.get(id)?;
    let address = if target == OpenTarget::Browser {
        sites
            .plan()?
            .entries
            .into_iter()
            .find(|entry| entry.project_id == id)
            .and_then(|entry| entry.address)
    } else {
        None
    };
    let command = opener::command_here(
        target,
        address.as_deref(),
        std::path::Path::new(&project.path),
    )?;
    opener::spawn(&command)
}

/// How to connect to the database lunaRun runs for a project (`root`, no password, the port lunaRun gave it), or
/// nothing when the project expects no MySQL or MariaDB or that is not installed. The password of the `.env` file is
/// never read.
#[tauri::command]
pub async fn project_database(
    id: String,
    launcher: State<'_, ProjectLauncher>,
) -> Result<Option<DatabaseSettings>, LunaError> {
    let launcher = launcher.inner().clone();
    blocking(move || launcher.database_settings(&id)).await
}

/// What would make the `.env` of a project connect to the database lunaRun runs: the **one line** of `DB_PORT` that would
/// change, as a preview, and the backup that would be made. Nothing when there is nothing to fix. Writes nothing.
#[tauri::command]
pub async fn project_env_port_preview(
    id: String,
    launcher: State<'_, ProjectLauncher>,
) -> Result<Option<PortChange>, LunaError> {
    let launcher = launcher.inner().clone();
    blocking(move || launcher.env_port_preview(&id)).await
}

/// Make the change the user confirmed in the preview: a backup of the `.env` file, then that one line. The page sends back
/// the preview it showed; nothing is written unless the file still gives exactly that preview (`ENV_CHANGED`).
#[tauri::command]
pub async fn project_env_port_apply(
    id: String,
    change: PortChange,
    launcher: State<'_, ProjectLauncher>,
) -> Result<EnvFixApplied, LunaError> {
    let launcher = launcher.inner().clone();
    blocking(move || launcher.env_port_apply(&id, &change)).await
}

/// Where the database the `.env` of a project names stands: not there, there, or not checkable (server not running).
#[tauri::command]
pub async fn project_database_status(
    id: String,
    launcher: State<'_, ProjectLauncher>,
) -> Result<DatabaseStatus, LunaError> {
    let launcher = launcher.inner().clone();
    blocking(move || launcher.database_status(&id)).await
}

/// Create the database the `.env` of a project names on the server lunaRun runs for it (`DATABASE_NOT_RUNNING` when it is
/// not running). Nothing changes when it exists.
#[tauri::command]
pub async fn project_database_create(
    id: String,
    launcher: State<'_, ProjectLauncher>,
) -> Result<DatabaseCreated, LunaError> {
    let launcher = launcher.inner().clone();
    blocking(move || launcher.database_create(&id)).await
}

/// Everything the first-run wizard shows (FR-AP-06, D-044): whether it is the first run, the data folder and where it came
/// from, the checks (Visual C++ on Windows, free space) and the starter pack for this computer.
#[tauri::command]
pub async fn onboarding_get(
    home: State<'_, LunaHome>,
    registry: State<'_, RegistryHolder>,
    data: State<'_, Arc<DetectionData>>,
    projects: State<'_, ProjectStore>,
) -> Result<OnboardingView, LunaError> {
    let (home, registry, data, projects) = (
        home.inner().clone(),
        registry.inner().clone(),
        Arc::clone(data.inner()),
        projects.inner().clone(),
    );
    blocking(move || {
        let today = CalDate::today()
            .ok_or_else(|| LunaError::internal("the system clock does not give a usable date"))?;
        let catalog = registry
            .available(Os::current(), Arch::current(), today)
            .runtimes;
        let installed = list_installed(&home)?;
        onboarding::overview(
            &home,
            Os::current(),
            &catalog,
            &installed,
            projects.list().len(),
            &data,
            || {
                luna_core::runtimes::prerequisites::detect_vcredist(
                    &luna_core::runtimes::prerequisites::default_reg_path(),
                )
            },
            onboarding::free_space,
        )
    })
    .await
}

/// The wizard was finished or skipped: it is not shown again.
#[tauri::command]
pub fn onboarding_complete(home: State<'_, LunaHome>) -> Result<(), LunaError> {
    onboarding::complete(&home)
}

/// Whether `path` can be the data folder, and what to know about it (a synchronised folder, an existing lunaRun folder).
/// Reads only: the page shows this before anything is written.
#[tauri::command]
pub async fn onboarding_check_home(path: String) -> Result<HomeChoice, LunaError> {
    blocking(move || onboarding::check_home(std::path::Path::new(&path))).await
}

/// Choose the data folder for the next start (first run only, and not when `LUNA_HOME` sets it). The app keeps its
/// current folder until `app_restart`.
#[tauri::command]
pub async fn onboarding_choose_home(
    path: String,
    home: State<'_, LunaHome>,
    projects: State<'_, ProjectStore>,
) -> Result<HomeChoice, LunaError> {
    let (home, projects) = (home.inner().clone(), projects.inner().clone());
    blocking(move || {
        let pointer = onboarding::pointer_file().ok_or_else(|| {
            LunaError::internal("this system has no configuration folder to remember the choice in")
        })?;
        let installed = list_installed(&home)?.len();
        onboarding::choose_home_now(
            &home,
            installed,
            projects.list().len(),
            std::path::Path::new(&path),
            &pointer,
        )
    })
    .await
}

/// Start lunaRun again (after a new data folder was chosen). Refused while a service runs: a restart would not stop it in
/// order.
#[tauri::command]
pub fn app_restart(
    app: tauri::AppHandle,
    supervisor: State<'_, Supervisor>,
) -> Result<(), LunaError> {
    if supervisor
        .list()
        .iter()
        .any(|status| status.state.has_process())
    {
        return Err(LunaError::new(
            luna_core::ErrorCode::Busy,
            "lunaRun cannot restart while a service runs: stop it first",
        ));
    }
    app.restart()
}

/// The settings, their defaults and the data folder (FR-AP-03, D-045).
#[tauri::command]
pub fn settings_get(home: State<'_, LunaHome>) -> Result<SettingsView, LunaError> {
    settings::view(&home)
}

/// Check and store the settings. A refused value writes nothing (`INVALID_INPUT` with `details.field`, `INVALID_URL`).
/// The defaults of new projects apply at once, new port ranges after a restart, a new address on the next refresh.
#[tauri::command]
pub fn settings_set(
    settings: Settings,
    home: State<'_, LunaHome>,
) -> Result<SettingsView, LunaError> {
    settings::save(&home, &settings)?;
    settings::view(&home)
}

/// Whether lunaRun starts when the user logs in. The operating system is asked (a Run key, a LaunchAgent, an autostart
/// file): nothing is stored by lunaRun.
#[tauri::command]
pub fn launch_at_login_get(app: tauri::AppHandle) -> Result<bool, LunaError> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|err| {
        LunaError::internal(format!(
            "could not ask the system about launch at login: {err}"
        ))
    })
}

/// Start lunaRun at login (hidden, in the tray, D-043) or not. Returns what the system says afterwards.
#[tauri::command]
pub fn launch_at_login_set(enabled: bool, app: tauri::AppHandle) -> Result<bool, LunaError> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let changed = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    changed.map_err(|err| {
        LunaError::new(
            luna_core::ErrorCode::PermissionDenied,
            format!("the system did not accept the change to launch at login: {err}"),
        )
    })?;
    autolaunch.is_enabled().map_err(|err| {
        LunaError::internal(format!(
            "could not ask the system about launch at login: {err}"
        ))
    })
}

/// After a change to a project: give Caddy the sites. A problem does not fail the change the user made (the project is
/// added, changed or removed either way): the manager keeps it and `sites_list` shows it.
async fn apply_sites(sites: &SiteManager) {
    let _ = sites.sync().await;
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
