//! Start, stop and restart a service by the id of its row on the Services page (FR-SV-01 to FR-SV-03, FR-SV-10; D-032).
//!
//! The catalog ([`list_services`](super::list_services)) says what can run and in what state. This is the other half:
//! it turns a row id into what has to happen, **with no knowledge of Tauri**, so everything is tested with the stand-in
//! service and fake runtimes, and the app's commands only forward.
//!
//! - **A database** (`mysql@8.4`): its definition from the newest installed patch of the line; if its data folder is
//!   not there yet the **first start sets it up** (slow, so it reports the phase `setup` and can be cancelled), then
//!   the supervisor starts it.
//! - **A PHP pool** (`php-fcgi@8.3.36`, `php-fpm@8.3.36`): every process of the pool, **all or none**: a worker that
//!   cannot start stops the ones already started, so a pool is never left half up.
//! - **Caddy**: its Caddyfile holds **the sites of the projects** ([`with_sites`](ServiceController::with_sites): the
//!   project store's answer, so `services` does not depend on `projects`) and is validated first. Without a source of
//!   sites only its admin API runs.
//!
//! "Started" means **running**: the call returns when the service answers its ready check, and fails with what the
//! supervisor knows about why it did not (`SERVICE_START_FAILED`, `PORT_IN_USE`, ...). [`ServiceController::run`] wraps
//! one action as a background job (`service:<id>`), so a first start shows progress and can be cancelled.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use super::catalog::{ServiceKind, source_of};
use super::databases::DatabaseDefinition;
use super::logs::LogLine;
use super::php::{PhpDefinition, pool_id, pool_kind, pool_members, pool_size};
use super::ports::{PortAllocator, PortPurpose, check_ports};
use super::state::{ServiceState, ServiceStatus};
use super::supervisor::Supervisor;
use super::{CaddyDefinition, Site, caddy_definition, database_definition, php_definition};
use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};
use crate::jobs::{JobContext, JobManager, StartedJob};
use crate::paths::LunaHome;
use crate::registry::RuntimeVersion;
use crate::runtimes::{InstalledRuntime, list_installed};

/// A database's setup program gets this long. The first start of MySQL 9.7 took 8 s and MariaDB 12.3 5 s on a slow PC;
/// a slow disk or a scanner can multiply that.
pub const SETUP_LIMIT: Duration = Duration::from_secs(300);
/// The allocator's key of a site's port is this followed by the project's id.
pub const SITE_PREFIX: &str = "site@";
/// `caddy validate` reads one small file.
const VALIDATE_LIMIT: Duration = Duration::from_secs(30);
/// Slack on top of a spec's own start timeout before giving up on waiting for it to settle: the supervisor times the
/// start out itself, and this only keeps a bug there from hanging a job for ever.
const SETTLE_MARGIN: Duration = Duration::from_secs(15);

/// The most lines of one log source `ServiceController::logs` returns: what the supervisor keeps (`LogConfig::ring_lines`).
pub const MAX_LOG_TAIL: usize = 2_000;

/// What one log source of a service has printed, oldest line first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceLines {
    /// `output`, or the label of a log file the service writes itself (`MySQL error log`).
    pub source: String,
    pub lines: Vec<LogLine>,
}

/// The log of one service (a row has several for a Windows PHP pool).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceLogs {
    /// The supervisor's id: the row's own, or `php-fcgi@8.3.36-w2`. It is the `id` of the `service:log` events.
    pub service: String,
    pub sources: Vec<SourceLines>,
}

/// `php-fcgi@8.3.36-w2` is a worker of the pool `php-fcgi@8.3.36`; every other id is its own row.
fn row_of_service(service: &str) -> &str {
    match service.rsplit_once("-w") {
        Some((pool, number))
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) =>
        {
            pool
        }
        _ => service,
    }
}

/// The job phases, which the page turns into words.
pub mod phase {
    /// A database's first start: creating its data folder.
    pub const SETUP: &str = "setup";
    /// The process was launched and is being waited for.
    pub const STARTING: &str = "starting";
    /// Asking the processes to leave.
    pub const STOPPING: &str = "stopping";
}

/// What one of the actions of a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

impl ServiceAction {
    /// The start of a job label (`Start mysql@8.4`): the page tells the actions apart by it, as it does for installing
    /// and uninstalling a runtime.
    pub fn label_prefix(self) -> &'static str {
        match self {
            Self::Start => "Start ",
            Self::Stop => "Stop ",
            Self::Restart => "Restart ",
        }
    }
}

/// The job key of a row: starting, stopping and restarting one service can never run at the same time.
pub fn job_key(id: &str) -> String {
    format!("service:{id}")
}

/// Everything needed to start one row, **built** (config files written, ports assigned) but nothing started yet.
/// The fields are public so a test can put a stand-in in place of the real program (see
/// [`ServiceController::with_plan_hook`]).
#[derive(Debug, Clone)]
pub enum StartPlan {
    Database(DatabaseDefinition),
    Php(PhpDefinition),
    Web(CaddyDefinition),
}

impl StartPlan {
    /// The supervisor's ids of the services this starts (a Windows pool has one per worker).
    pub fn service_ids(&self) -> Vec<String> {
        match self {
            Self::Database(def) => vec![def.spec.id.clone()],
            Self::Php(def) => def.specs.iter().map(|spec| spec.id.clone()).collect(),
            Self::Web(def) => vec![def.spec.id.clone()],
        }
    }
}

type PlanHook = Arc<dyn Fn(&mut StartPlan) + Send + Sync>;
type QuitCheck = Arc<dyn Fn() -> bool + Send + Sync>;
/// Where the controller gets the sites Caddy serves when it plans Caddy.
pub type SiteSource = Arc<dyn Fn() -> Result<Vec<Site>> + Send + Sync>;

/// Starts, stops and restarts services by row id. Cheap to clone.
#[derive(Clone)]
pub struct ServiceController {
    home: LunaHome,
    ports: PortAllocator,
    supervisor: Supervisor,
    os: Os,
    quitting: QuitCheck,
    hook: Option<PlanHook>,
    sites: Option<SiteSource>,
}

fn busy(message: String, id: &str) -> LunaError {
    LunaError::new(ErrorCode::Busy, message).with_details(json!({ "service": id }))
}

fn not_found(message: String, id: &str) -> LunaError {
    LunaError::new(ErrorCode::NotFound, message).with_details(json!({ "service": id }))
}

fn cancelled(id: &str) -> LunaError {
    LunaError::new(ErrorCode::Cancelled, format!("starting {id} was cancelled"))
}

impl ServiceController {
    /// `os` is a parameter so a Windows pool and a PHP-FPM are both tested on any machine. Pass `Os::current()`.
    pub fn new(home: LunaHome, ports: PortAllocator, supervisor: Supervisor, os: Os) -> Self {
        Self {
            home,
            ports,
            supervisor,
            os,
            quitting: Arc::new(|| false),
            hook: None,
            sites: None,
        }
    }

    /// The sites Caddy serves, asked for each time Caddy is planned (a start, a restart, adopting it), so a project added
    /// since is served without anything else being told.
    #[must_use]
    pub fn with_sites(
        mut self,
        sites: impl Fn() -> Result<Vec<Site>> + Send + Sync + 'static,
    ) -> Self {
        self.sites = Some(Arc::new(sites));
        self
    }

    /// Nothing is started while `quitting` says the app is quitting (`BUSY`, D-029): the orderly stop has begun, and a
    /// service started now would outlive it.
    #[must_use]
    pub fn with_quit_check(mut self, quitting: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.quitting = Arc::new(quitting);
        self
    }

    /// Change every plan after it is built and before it is started. For tests: the real programs are not there.
    #[must_use]
    pub fn with_plan_hook(mut self, hook: impl Fn(&mut StartPlan) + Send + Sync + 'static) -> Self {
        self.hook = Some(Arc::new(hook));
        self
    }

    /// The supervisor's ids behind a row: the row itself, or a Windows pool's workers. Nothing is read from what is
    /// installed, so a service can be stopped whatever happened to its runtime.
    ///
    /// # Errors
    /// `NOT_FOUND` for an id that is not a service of this system.
    pub fn members(&self, id: &str) -> Result<Vec<String>> {
        let unknown = || not_found(format!("there is no service {id}"), id);
        if id == "caddy" {
            return Ok(vec![id.to_owned()]);
        }
        let (kind, version) = id.split_once('@').ok_or_else(unknown)?;
        match kind {
            "mysql" | "mariadb" => {
                let valid = version.split('.').count() == 2
                    && version
                        .split('.')
                        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
                if valid {
                    Ok(vec![id.to_owned()])
                } else {
                    Err(unknown())
                }
            }
            _ if kind == pool_kind(self.os) && RuntimeVersion::parse(version).is_some() => {
                pool_members(&self.ports, self.os, id)
            }
            _ => Err(unknown()),
        }
    }

    fn refuse_if_quitting(&self, id: &str) -> Result<()> {
        if (self.quitting)() {
            return Err(busy(
                "lunaRun is quitting and is stopping its services: nothing can be started now"
                    .to_owned(),
                id,
            ));
        }
        Ok(())
    }

    /// The status of each of `ids`; a service the supervisor has never seen is stopped.
    fn statuses(&self, ids: &[String]) -> Vec<ServiceStatus> {
        ids.iter()
            .map(|id| {
                self.supervisor
                    .status(id)
                    .unwrap_or_else(|| ServiceStatus::stopped(id))
            })
            .collect()
    }

    /// Build what it takes to start row `id`: config files written and ports assigned, **nothing started**.
    ///
    /// # Errors
    /// `NOT_FOUND` (not a service of this system, or its runtime is not installed), `BUSY` (a service of the row
    /// already runs, which must not have its config rewritten under it), and what the definitions refuse with:
    /// `RUNTIME_BROKEN`, `INVALID_PATH`, `NO_FREE_PORT`, `PORT_IN_USE`, I/O errors.
    pub fn plan(&self, id: &str) -> Result<StartPlan> {
        let members = self.members(id)?;
        if let Some(running) = self
            .statuses(&members)
            .into_iter()
            .find(|status| status.state.has_process())
        {
            return Err(busy(
                format!("{} is already {}", running.id, describe(running.state)),
                &running.id,
            ));
        }
        let installed = list_installed(&self.home)?;
        let (kind, runtime) = source_of(&installed, id, self.os).ok_or_else(|| {
            not_found(
                format!("{id} cannot be started: the version it runs from is not installed (see Runtimes)"),
                id,
            )
        })?;
        let mut plan = self.build(kind, runtime)?;
        if let Some(hook) = &self.hook {
            hook(&mut plan);
        }
        Ok(plan)
    }

    fn build(&self, kind: ServiceKind, runtime: &InstalledRuntime) -> Result<StartPlan> {
        Ok(match kind {
            ServiceKind::Database => StartPlan::Database(database_definition(
                &self.home,
                &self.ports,
                runtime,
                self.os,
            )?),
            ServiceKind::Php => {
                let pool = pool_id(self.os, &runtime.version);
                let workers = pool_size(&self.ports, self.os, &pool)?;
                StartPlan::Php(php_definition(
                    &self.home,
                    &self.ports,
                    runtime,
                    self.os,
                    workers,
                )?)
            }
            ServiceKind::Web => {
                let sites = match &self.sites {
                    Some(source) => source()?,
                    None => Vec::new(),
                };
                StartPlan::Web(caddy_definition(
                    &self.home,
                    &self.ports,
                    runtime,
                    &sites,
                    self.os,
                )?)
            }
        })
    }

    /// Start row `id` and return when it is **running**. `on_phase` is told `setup` (a database's first start) and
    /// `starting`. When `cancel` fires, or when it does not come up, everything this started is stopped again.
    ///
    /// # Errors
    /// What [`plan`](Self::plan) refuses with, plus `BUSY` while the app quits, `PORT_IN_USE` (with `details`: port,
    /// pid, processName, processPath, reason), `DATABASE_INIT_FAILED` (`details.tail` is the end of the setup
    /// output), `CONFIG_INVALID`, `SERVICE_START_FAILED` (the supervisor's own reason) and `CANCELLED`.
    pub async fn start(
        &self,
        id: &str,
        cancel: &CancellationToken,
        on_phase: &(dyn Fn(&str) + Sync),
    ) -> Result<()> {
        self.refuse_if_quitting(id)?;
        let plan = self.plan(id)?;
        match &plan {
            StartPlan::Database(def) => {
                if def.needs_initialisation() {
                    on_phase(phase::SETUP);
                    def.initialise(SETUP_LIMIT, cancel).await?;
                }
            }
            StartPlan::Php(def) => {
                // The whole pool or none of it: look at every port before starting the first worker.
                check_ports(&def.ports)?;
            }
            StartPlan::Web(def) => {
                on_phase(phase::STARTING);
                def.validate(VALIDATE_LIMIT, cancel).await?;
            }
        }
        // The app may have begun to quit while a first start was setting up.
        self.refuse_if_quitting(id)?;
        on_phase(phase::STARTING);
        let specs = match plan {
            StartPlan::Database(def) => vec![def.spec],
            StartPlan::Php(def) => def.specs,
            StartPlan::Web(def) => vec![def.spec],
        };
        let ids: Vec<String> = specs.iter().map(|spec| spec.id.clone()).collect();
        let limit = specs
            .iter()
            .map(|spec| spec.start_timeout)
            .max()
            .unwrap_or(Duration::ZERO)
            + SETTLE_MARGIN;

        for (index, spec) in specs.into_iter().enumerate() {
            if let Err(err) = self.supervisor.start(spec) {
                // Not started, so nothing of this one to stop: only the workers before it.
                self.stop_running(&ids[..index]).await;
                return Err(err);
            }
        }
        self.wait_until_running(&ids, limit, cancel).await
    }

    /// Wait until every service of `ids` runs. If one cannot (or `cancel` fires) the ones that do run are stopped
    /// again; a service that failed keeps its `Error` state and message on the page.
    async fn wait_until_running(
        &self,
        ids: &[String],
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Result<()> {
        for id in ids {
            let settled = tokio::select! {
                settled = self.supervisor.wait_until_settled(id, limit) => settled,
                () = cancel.cancelled() => {
                    self.stop_running(ids).await;
                    return Err(cancelled(&ids.join(", ")));
                }
            };
            let status = match settled {
                Ok(status) => status,
                Err(err) => {
                    self.stop_running(ids).await;
                    return Err(err);
                }
            };
            if status.state != ServiceState::Running {
                self.stop_running(ids).await;
                return Err(match status.state {
                    ServiceState::Stopped => cancelled(id),
                    _ => status.error.unwrap_or_else(|| {
                        LunaError::new(ErrorCode::ServiceStartFailed, format!("{id} did not start"))
                            .with_details(json!({ "service": id }))
                    }),
                });
            }
        }
        Ok(())
    }

    /// Stop those of `ids` that have a process. Best effort, for undoing a start.
    async fn stop_running(&self, ids: &[String]) {
        let running: Vec<String> = self
            .statuses(ids)
            .into_iter()
            .filter(|status| status.state.has_process())
            .map(|status| status.id)
            .collect();
        join_all(running.iter().map(|id| self.supervisor.stop(id))).await;
    }

    /// Stop every service of row `id` and return when all are stopped. Stopping what is not running does nothing, and
    /// a service that crashed or failed to start just becomes `Stopped` (the page's "Stop" clears the failure).
    ///
    /// # Errors
    /// `NOT_FOUND` for an id that is not a service of this system, and the first error of a stop.
    pub async fn stop(&self, id: &str) -> Result<()> {
        let members = self.members(id)?;
        let results = join_all(members.iter().map(|member| async move {
            match self.supervisor.stop(member).await {
                // Never started in this run: there is nothing to stop.
                Err(err) if err.code == ErrorCode::NotFound => Ok(()),
                other => other,
            }
        }))
        .await;
        results.into_iter().collect()
    }

    /// Stop row `id`, then start it again.
    ///
    /// # Errors
    /// As [`stop`](Self::stop) and [`start`](Self::start).
    pub async fn restart(
        &self,
        id: &str,
        cancel: &CancellationToken,
        on_phase: &(dyn Fn(&str) + Sync),
    ) -> Result<()> {
        self.refuse_if_quitting(id)?;
        on_phase(phase::STOPPING);
        self.stop(id).await?;
        self.start(id, cancel, on_phase).await
    }

    /// "Use another port": give row `id` other ports than it has (the old ones are taken, or the user wants to move),
    /// and start it. The service must not be running.
    ///
    /// For **Caddy** the ports that can be taken are the ones of its **sites** (one per project): every site whose port
    /// is taken now is moved to another one, and the Caddyfile written at the start has the new ports. Only when none
    /// is taken is it the admin port, which is one port with no other to offer (the error names who has it).
    ///
    /// # Errors
    /// `BUSY` while it runs or the app quits, `PORT_IN_USE` for Caddy's admin port (it is one port and there is no
    /// other to offer: the error names who has it), `NO_FREE_PORT`, `NOT_FOUND`, and what a start refuses with.
    pub fn reassign_and_start(&self, jobs: &JobManager, id: &str) -> Result<StartedJob> {
        self.refuse_if_quitting(id)?;
        let members = self.members(id)?;
        if let Some(running) = self
            .statuses(&members)
            .into_iter()
            .find(|status| status.state.has_process())
        {
            return Err(busy(
                format!(
                    "{} is {}: stop it before moving it to another port",
                    running.id,
                    describe(running.state)
                ),
                &running.id,
            ));
        }
        if !(id == "caddy" && self.move_taken_sites()?) {
            self.ports.reassign(id, self.port_purpose(id)?)?;
        }
        self.run(jobs, id, ServiceAction::Start)
    }

    /// Give every site whose port is taken another port. `true` when at least one was moved.
    fn move_taken_sites(&self) -> Result<bool> {
        let mut moved = false;
        for (key, assignment) in self.ports.all()? {
            if key.starts_with(SITE_PREFIX) && check_ports(&assignment.ports()).is_err() {
                self.ports.reassign(&key, PortPurpose::Site)?;
                moved = true;
            }
        }
        Ok(moved)
    }

    /// What the ports of row `id` are for (the allocator keys them by the id, a pool by the pool id).
    fn port_purpose(&self, id: &str) -> Result<PortPurpose> {
        if id == "caddy" {
            return Ok(PortPurpose::CaddyAdmin);
        }
        if id.starts_with("mysql@") || id.starts_with("mariadb@") {
            return Ok(PortPurpose::Database);
        }
        Ok(PortPurpose::PhpFastCgi {
            pool: pool_size(&self.ports, self.os, id)?,
        })
    }

    /// What a row's services have printed: for each service of the row its sources (`output` first), each with its
    /// last `tail` lines (at most [`MAX_LOG_TAIL`]), oldest first. The lines of a service that has stopped are still
    /// there until it is started again; a service that was never started has no sources yet.
    ///
    /// # Errors
    /// `NOT_FOUND` for an id that is not a service of this system.
    pub fn logs(&self, id: &str, tail: usize) -> Result<Vec<ServiceLogs>> {
        let tail = tail.min(MAX_LOG_TAIL);
        Ok(self
            .members(id)?
            .into_iter()
            .map(|service| {
                let sources = self
                    .supervisor
                    .log_sources(&service)
                    .into_iter()
                    .map(|source| SourceLines {
                        lines: self.supervisor.log_lines(&service, &source, tail),
                        source,
                    })
                    .collect();
                ServiceLogs { service, sources }
            })
            .collect())
    }

    /// "Adopt": take over a service an earlier run of lunaRun left running (`Supervisor::scan_leftovers` found it).
    /// The definition is built as for a start, so the adopted service is checked, stopped and restarted exactly as
    /// one this app started. `service` is the supervisor's id (what `Leftover::id` holds).
    ///
    /// # Errors
    /// `BUSY` while the app quits, `NOT_FOUND` when no such service is left running or the version it runs from is no
    /// longer installed (then it can only be stopped), `INVALID_PATH` when what runs is not the program the
    /// definition names, `SERVICE_START_FAILED` when it does not answer its ready check (it is left running).
    pub async fn adopt(&self, service: &str) -> Result<()> {
        self.refuse_if_quitting(service)?;
        let plan = self.plan(row_of_service(service))?;
        let specs = match plan {
            StartPlan::Database(def) => vec![def.spec],
            StartPlan::Php(def) => def.specs,
            StartPlan::Web(def) => vec![def.spec],
        };
        let spec = specs
            .into_iter()
            .find(|spec| spec.id == service)
            .ok_or_else(|| not_found(format!("there is no service {service}"), service))?;
        self.supervisor.adopt(spec).await
    }

    /// "Stop" for a leftover: ask it to leave, end it after ten seconds, end what it left behind, forget it.
    ///
    /// # Errors
    /// `NOT_FOUND` when no such service is left running.
    pub async fn stop_leftover(&self, service: &str) -> Result<()> {
        self.members(row_of_service(service))?;
        self.supervisor.stop_leftover(service).await
    }

    /// Do `action` on row `id` as a background job named `service:<id>` (`job:progress` and `job:done` events), and
    /// return at once. The job succeeds when a start or restart has the service **running**.
    ///
    /// Asking again for the action that is already running returns that job (`already_running`). Asking for
    /// another action on a service whose job is still running is `BUSY`: stop and start must not run side by side.
    ///
    /// # Errors
    /// `NOT_FOUND` for an id that is not a service of this system, `BUSY` while the app quits (for a start or restart)
    /// or while another job of this service runs. Everything else is the job's own result.
    pub fn run(&self, jobs: &JobManager, id: &str, action: ServiceAction) -> Result<StartedJob> {
        self.members(id)?;
        if action != ServiceAction::Stop {
            self.refuse_if_quitting(id)?;
        }
        let label = format!("{}{id}", action.label_prefix());
        let controller = self.clone();
        let service = id.to_owned();
        let started = jobs.start(&job_key(id), &label, move |ctx: JobContext| async move {
            let cancel = ctx.cancel_token().clone();
            let on_phase = |phase: &str| ctx.phase(phase, None);
            match action {
                ServiceAction::Start => controller.start(&service, &cancel, &on_phase).await,
                ServiceAction::Restart => controller.restart(&service, &cancel, &on_phase).await,
                ServiceAction::Stop => {
                    on_phase(phase::STOPPING);
                    controller.stop(&service).await
                }
            }
        });
        if started.already_running && jobs.get(&started.id).is_some_and(|job| job.label != label) {
            return Err(busy(
                format!("{id} is busy with another action: wait for it to finish"),
                id,
            ));
        }
        Ok(started)
    }
}

fn describe(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Starting => "starting",
        ServiceState::Running => "running",
        ServiceState::Stopping => "stopping",
        ServiceState::Stopped => "stopped",
        ServiceState::Crashed => "crashed",
        ServiceState::Error => "in error",
    }
}
