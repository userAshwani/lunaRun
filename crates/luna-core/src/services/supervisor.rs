//! The supervisor: owns every service's process and its [`ServiceState`] (docs/SERVICES.md section 2).
//!
//! Each started service gets one **lifecycle task** that launches the process, waits for it to be ready, watches
//! it, restarts it after a crash (the [`RestartPolicy`](super::RestartPolicy)) and stops it when asked. That task
//! is the only writer of the service's state while it lives, so there is never a race between "it crashed" and
//! "it was stopped". Every change is applied through the pure table in `state.rs` and announced to a
//! [`ServiceEventSink`].
//!
//! - **No orphans (FR-SV-07).** Every process is started through a [`Containment`] (a Job Object on Windows, a
//!   process group elsewhere) and has a PID file while it runs. After a crash of lunaRun,
//!   [`scan_leftovers`](Supervisor::scan_leftovers) finds the services that are still running, and each can be
//!   [adopted](Supervisor::adopt) (tracked again, with the same lifecycle) or [stopped](Supervisor::stop_leftover).
//! - **Sinks must not call back into the supervisor.** Events are sent while the state lock is held, which keeps
//!   them in order, and before the new status is visible to [`Supervisor::status`] and the `wait_until` family. The
//!   Tauri sink only emits to the webview.
//! - **Sleeping and timing use real time.** Policies are plain values (`RestartPolicy`, `start_timeout`,
//!   `StopPlan::grace`), so tests give them milliseconds.
//! - **Not here yet:** log capture (M1.4 task 3), ports (task 4), the service definitions (task 5) and the quit
//!   order (task 6).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Child;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use super::containment::{Containment, OsContainment};
use super::os;
use super::pidfile::{self, PidRecord};
use super::process;
use super::restart::{Decision, RestartTracker};
use super::spec::{ReadyCheck, ServiceSpec, StopPlan};
use super::state::{Event, ServiceState, ServiceStatus};
use crate::error::{ErrorCode, LunaError, Result};
use crate::paths::LunaHome;

/// Receives a snapshot of a service every time its state changes. Closures work too.
pub trait ServiceEventSink: Send + Sync + 'static {
    fn emit(&self, status: ServiceStatus);
}

impl<F: Fn(ServiceStatus) + Send + Sync + 'static> ServiceEventSink for F {
    fn emit(&self, status: ServiceStatus) {
        self(status);
    }
}

/// A service that is still running from an earlier run of lunaRun (found through its PID file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Leftover {
    /// The service id, for example `mysql@8.4`.
    pub id: String,
    pub pid: u32,
    /// The program it was started from.
    pub exe: String,
    /// Seconds since 1970-01-01 UTC.
    #[ts(type = "number")]
    pub started_at: u64,
    pub port: Option<u16>,
}

struct Entry {
    status: watch::Sender<ServiceStatus>,
    /// Set while a lifecycle task is alive. Cancelling it asks the service to stop.
    active: Option<CancellationToken>,
}

struct Inner {
    handle: Handle,
    home: LunaHome,
    sink: Arc<dyn ServiceEventSink>,
    containment: Arc<dyn Containment>,
    /// How often a ready check is repeated, and how often an adopted process is checked for being alive.
    poll: Duration,
    services: Mutex<HashMap<String, Entry>>,
}

/// Owns and supervises services. Cheap to clone.
#[derive(Clone)]
pub struct Supervisor(Arc<Inner>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The maps hold plain state that stays consistent after any single update, and one panicking
    // service must not take every other service's control down with it.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Supervisor {
    /// A supervisor that contains every process in a Job Object (Windows) or a process group (macOS, Linux).
    pub fn new(handle: Handle, home: LunaHome, sink: impl ServiceEventSink) -> Self {
        Self(Arc::new(Inner {
            handle,
            home,
            sink: Arc::new(sink),
            containment: Arc::new(OsContainment::new()),
            poll: Duration::from_millis(100),
            services: Mutex::new(HashMap::new()),
        }))
    }

    /// Use another [`Containment`] for every process started from now on.
    #[must_use]
    pub fn with_containment(self, containment: Arc<dyn Containment>) -> Self {
        self.modified(|inner| inner.containment = containment)
    }

    /// How often a ready check is repeated (default 100 ms).
    #[must_use]
    pub fn with_poll_interval(self, poll: Duration) -> Self {
        self.modified(|inner| inner.poll = poll)
    }

    fn modified(self, change: impl FnOnce(&mut Inner)) -> Self {
        // Only meant for setup, before any service exists: services started earlier keep what they had.
        let old = &*self.0;
        let mut inner = Inner {
            handle: old.handle.clone(),
            home: old.home.clone(),
            sink: Arc::clone(&old.sink),
            containment: Arc::clone(&old.containment),
            poll: old.poll,
            services: Mutex::new(std::mem::take(&mut *lock(&old.services))),
        };
        change(&mut inner);
        Self(Arc::new(inner))
    }

    /// Start a service. Returns as soon as the spec is accepted and the service is `Starting`; follow it with
    /// [`status`](Self::status), the sink, or [`wait_until_settled`](Self::wait_until_settled).
    ///
    /// - A spec that fails [`ServiceSpec::validate`] is refused with nothing started (`PATH_OUTSIDE_ROOT`, ...).
    /// - A service that is starting, running or stopping is refused with `BUSY`.
    /// - One that is stopped, crashed or in error is started afresh (the restart count starts again at 0).
    pub fn start(&self, spec: ServiceSpec) -> Result<()> {
        spec.validate(&self.0.home)?;
        let token = self.begin(&spec.id, |_| {})?;
        self.launch(spec, token, None);
        Ok(())
    }

    /// Mark the service `Starting` and give it a lifecycle token. `after` runs on the new status.
    fn begin(&self, id: &str, after: impl FnOnce(&mut ServiceStatus)) -> Result<CancellationToken> {
        let token = CancellationToken::new();
        let mut services = lock(&self.0.services);
        let entry = services.entry(id.to_owned()).or_insert_with(|| {
            let (status, _) = watch::channel(ServiceStatus::stopped(id));
            Entry {
                status,
                active: None,
            }
        });
        if entry.active.is_some() || entry.status.borrow().state.has_process() {
            return Err(LunaError::new(
                ErrorCode::Busy,
                format!("{id} is already {}", describe(entry.status.borrow().state)),
            )
            .with_details(json!({ "service": id })));
        }
        // A fresh run: forget how the last one ended (the state itself is set by the event).
        apply(&self.0, entry, Event::Start, |status| {
            status.pid = None;
            status.restarts = 0;
            status.exit_code = None;
            status.error = None;
            status.gave_up = false;
            after(status);
        });
        entry.active = Some(token.clone());
        Ok(token)
    }

    /// Spawn the lifecycle task. `adopted` is the number of a process that is already running.
    fn launch(&self, spec: ServiceSpec, token: CancellationToken, adopted: Option<u32>) {
        let inner = Arc::clone(&self.0);
        self.0.handle.spawn(async move {
            let _finish = Finish {
                inner: Arc::clone(&inner),
                id: spec.id.clone(),
                token: token.clone(),
            };
            run(&inner, &spec, &token, adopted).await;
        });
    }

    /// Stop a service and return once it is `Stopped`. Stopping a stopped service does nothing; an unknown id is
    /// `NOT_FOUND`. A service that crashed or failed to start (nothing is running) just becomes `Stopped`.
    pub async fn stop(&self, id: &str) -> Result<()> {
        let mut updates = {
            let mut services = lock(&self.0.services);
            let entry = services.get_mut(id).ok_or_else(|| not_found(id))?;
            match &entry.active {
                Some(token) => token.cancel(),
                None if entry.status.borrow().state == ServiceState::Stopped => return Ok(()),
                None => {
                    apply(&self.0, entry, Event::Stop, |_| {});
                    return Ok(());
                }
            }
            entry.status.subscribe()
        };
        updates
            .wait_for(|status| status.state == ServiceState::Stopped)
            .await
            .map(|_| ())
            .map_err(|_| {
                LunaError::internal(format!("{id} was dropped while it was being stopped"))
            })
    }

    /// Stop the service if it runs, then start it with `spec`.
    pub async fn restart(&self, spec: ServiceSpec) -> Result<()> {
        // Validate first, so a bad spec does not stop a service that is working.
        spec.validate(&self.0.home)?;
        self.stop(&spec.id).await.or_else(|err| {
            // Never started before: nothing to stop.
            if err.code == ErrorCode::NotFound {
                Ok(())
            } else {
                Err(err)
            }
        })?;
        self.start(spec)
    }

    pub fn status(&self, id: &str) -> Option<ServiceStatus> {
        lock(&self.0.services)
            .get(id)
            .map(|entry| entry.status.borrow().clone())
    }

    /// Every service that was ever started in this run, by id.
    pub fn list(&self) -> Vec<ServiceStatus> {
        let mut all: Vec<ServiceStatus> = lock(&self.0.services)
            .values()
            .map(|entry| entry.status.borrow().clone())
            .collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        all
    }

    /// Wait until the service has settled (see [`ServiceStatus::is_settled`]): running, stopped, failed to
    /// start, or out of restarts. `NOT_FOUND` for an unknown id, `INTERNAL` if it does not settle in `limit`.
    pub async fn wait_until_settled(&self, id: &str, limit: Duration) -> Result<ServiceStatus> {
        self.wait_until(id, limit, ServiceStatus::is_settled).await
    }

    /// Wait until `done` is true for the service's status.
    pub async fn wait_until(
        &self,
        id: &str,
        limit: Duration,
        done: impl Fn(&ServiceStatus) -> bool,
    ) -> Result<ServiceStatus> {
        let mut updates = {
            let services = lock(&self.0.services);
            services
                .get(id)
                .ok_or_else(|| not_found(id))?
                .status
                .subscribe()
        };
        match timeout(limit, updates.wait_for(|status| done(status))).await {
            Ok(Ok(status)) => Ok(status.clone()),
            Ok(Err(_)) => Err(LunaError::internal(format!(
                "{id} was dropped while waiting for it"
            ))),
            Err(_) => Err(LunaError::internal(format!(
                "{id} did not settle within {} ms",
                limit.as_millis()
            ))),
        }
    }

    // -----------------------------------------------------------------------------------------
    // After a crash of lunaRun
    // -----------------------------------------------------------------------------------------

    /// Look for services an earlier run of lunaRun left running (FR-SV-07). For every PID file: if the process is
    /// alive **and is still the program that was started**, it is returned; otherwise the file is stale and is
    /// deleted. Services this supervisor is running itself are skipped. Call it once at startup, before starting
    /// anything.
    pub fn scan_leftovers(&self) -> Vec<Leftover> {
        let managed: HashSet<String> = lock(&self.0.services)
            .iter()
            .filter(|(_, entry)| entry.active.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let mut found = Vec::new();
        for id in pidfile::ids(&self.0.home) {
            if managed.contains(&id) {
                continue;
            }
            // A file from a newer lunaRun, or one that cannot be read right now, is left alone.
            let Ok(record) = pidfile::read(&self.0.home, &id) else {
                continue;
            };
            match record.filter(is_running_as_recorded) {
                Some(record) => found.push(Leftover {
                    id,
                    pid: record.pid,
                    exe: record.exe,
                    started_at: record.started_at,
                    port: record.port,
                }),
                None => pidfile::remove(&self.0.home, &id),
            }
        }
        found
    }

    /// The record of a service that is still running from an earlier run, if there is one.
    fn leftover_record(&self, id: &str) -> Option<PidRecord> {
        if lock(&self.0.services)
            .get(id)
            .is_some_and(|entry| entry.active.is_some())
        {
            return None;
        }
        pidfile::read(&self.0.home, id)
            .ok()
            .flatten()
            .filter(is_running_as_recorded)
    }

    /// Stop a leftover service (the "Stop" choice): ask it to leave, kill it after ten seconds, kill what it
    /// left in its process group, and delete its PID file. `NOT_FOUND` when nothing like that is running.
    pub async fn stop_leftover(&self, id: &str) -> Result<()> {
        let record = self.leftover_record(id).ok_or_else(|| nothing_left(id))?;
        process::stop_pid(record.pid, &StopPlan::default()).await;
        os::kill_group(record.pid);
        pidfile::remove(&self.0.home, id);
        Ok(())
    }

    /// Take over a leftover service (the "Adopt" choice): it is tracked from now on with the same lifecycle as
    /// one this supervisor started (stop, crash detection, restart policy), except that it keeps running until
    /// then. `spec` says how to check, stop and restart it, and must name the program that is running.
    ///
    /// Refused with `NOT_FOUND` if no such process runs, with `INVALID_PATH` if it runs another program than
    /// `spec.program`, and with `SERVICE_START_FAILED` if it does not pass the ready check within
    /// `spec.start_timeout` (it is left untouched: the caller can stop it instead).
    pub async fn adopt(&self, spec: ServiceSpec) -> Result<()> {
        spec.validate(&self.0.home)?;
        let record = self
            .leftover_record(&spec.id)
            .ok_or_else(|| nothing_left(&spec.id))?;
        if !os::same_program(Path::new(&record.exe), &spec.program) {
            return Err(LunaError::new(
                ErrorCode::InvalidPath,
                format!(
                    "{} was started from {}, not from {}",
                    spec.id,
                    record.exe,
                    spec.program.display()
                ),
            )
            .with_details(json!({ "service": spec.id })));
        }
        if !process::wait_ready(&spec.ready, spec.start_timeout, self.0.poll).await {
            return Err(LunaError::new(
                ErrorCode::ServiceStartFailed,
                format!(
                    "{} is running but did not answer the ready check, so it was not adopted",
                    spec.id
                ),
            )
            .with_details(json!({ "service": spec.id, "pid": record.pid })));
        }
        let pid = record.pid;
        let token = self.begin(&spec.id, |status| status.pid = Some(pid))?;
        // It already runs: Starting is over before it began.
        update(&self.0, &spec.id, Event::Ready, |_| {});
        self.launch(spec, token, Some(pid));
        Ok(())
    }
}

/// The process is alive and is still the program the PID file says (the number may have been reused).
fn is_running_as_recorded(record: &PidRecord) -> bool {
    os::process_alive(record.pid)
        && os::process_exe(record.pid)
            .is_some_and(|exe| os::same_program(&exe, Path::new(&record.exe)))
}

fn nothing_left(id: &str) -> LunaError {
    LunaError::new(
        ErrorCode::NotFound,
        format!("no {id} is left running from an earlier run"),
    )
    .with_details(json!({ "service": id }))
}

fn not_found(id: &str) -> LunaError {
    LunaError::new(ErrorCode::NotFound, format!("there is no service {id}"))
        .with_details(json!({ "service": id }))
}

fn describe(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Starting => "starting",
        ServiceState::Stopping => "stopping",
        _ => "running",
    }
}

/// Apply `event` to the entry if the table allows it, let `edit` fill in the details, tell the sink, and only then
/// publish the new status to anyone waiting on it. In that order, a caller that wakes up because of a state can
/// rely on the sink having been told already. Returns whether the state changed. Called with the services lock
/// held, which is what makes the read-then-replace safe: every writer takes it.
fn apply(
    inner: &Inner,
    entry: &Entry,
    event: Event,
    edit: impl FnOnce(&mut ServiceStatus),
) -> bool {
    let mut next = entry.status.borrow().clone();
    let Some(state) = next.state.next(event) else {
        return false;
    };
    next.state = state;
    edit(&mut next);
    publish(inner, entry, next);
    true
}

/// Tell the sink, then publish to waiters (see [`apply`]).
fn publish(inner: &Inner, entry: &Entry, status: ServiceStatus) {
    inner.sink.emit(status.clone());
    entry.status.send_replace(status);
}

/// What a lifecycle task does to its service's entry.
fn update(inner: &Inner, id: &str, event: Event, edit: impl FnOnce(&mut ServiceStatus)) -> bool {
    let services = lock(&inner.services);
    services
        .get(id)
        .is_some_and(|entry| apply(inner, entry, event, edit))
}

/// Marks the lifecycle task as over, even if it panicked, so the service can be started again.
struct Finish {
    inner: Arc<Inner>,
    id: String,
    token: CancellationToken,
}

impl Drop for Finish {
    fn drop(&mut self) {
        let mut services = lock(&self.inner.services);
        if let Some(entry) = services.get_mut(&self.id) {
            entry.active = None;
            // A stop that arrived while the task was giving up (crashed, or a failed start) was never seen by
            // the task. Honour it here, or `stop` would wait for a `Stopped` that never comes.
            if self.token.is_cancelled() {
                apply(&self.inner, entry, Event::Stop, |status| status.pid = None);
            }
        }
    }
}

/// The process of a service: our own child, or one adopted after a crash of lunaRun that we can only watch by
/// its number.
enum Live {
    /// Boxed: a `Child` is much larger than a process number.
    Own(Box<Child>),
    Adopted(u32),
}

impl Live {
    fn pid(&self) -> Option<u32> {
        match self {
            Self::Own(child) => child.id(),
            Self::Adopted(pid) => Some(*pid),
        }
    }

    /// Resolves when the process has ended, with its exit code when we can know it.
    async fn ended(&mut self, poll: Duration) -> Option<i32> {
        match self {
            Self::Own(child) => exit_code(child.wait().await),
            Self::Adopted(pid) => {
                while os::process_alive(*pid) {
                    sleep(poll).await;
                }
                None
            }
        }
    }

    async fn stop(&mut self, plan: &StopPlan) {
        match self {
            Self::Own(child) => process::stop_child(child, plan).await,
            Self::Adopted(pid) => process::stop_pid(*pid, plan).await,
        }
    }

    async fn kill(&mut self) {
        match self {
            Self::Own(child) => process::kill_now(child).await,
            Self::Adopted(pid) => {
                os::terminate_process(*pid);
                os::wait_gone(*pid, Duration::from_secs(5)).await;
            }
        }
    }
}

/// How a launch went until the service was ready (or was not).
enum Launch {
    Ready,
    StopRequested,
    /// The process ended by itself before it was ready.
    Exited(Option<i32>),
    TimedOut,
}

/// How a running process ended.
enum Ended {
    StopRequested,
    Exited(Option<i32>),
}

/// What to do after a failure.
enum Next {
    /// Wait this long, then launch again.
    Restart(Duration),
    End,
}

fn exit_code(status: std::io::Result<std::process::ExitStatus>) -> Option<i32> {
    status.ok().and_then(|status| status.code())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The process is gone: kill what it left behind and delete its PID file. Done before the state changes, so
/// anyone who sees the new state also sees the files gone.
fn reaped(inner: &Inner, id: &str, pid: Option<u32>) {
    if let Some(pid) = pid {
        inner.containment.after_exit(pid);
    }
    pidfile::remove(&inner.home, id);
}

/// Record the process so that a later launch can find it if lunaRun dies first. Best effort: without the file the
/// service still works, it only loses the safety net.
fn record_started(inner: &Inner, spec: &ServiceSpec, pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let record = PidRecord {
        pid,
        exe: spec.program.display().to_string(),
        started_at: now_unix(),
        port: match spec.ready {
            ReadyCheck::Tcp { port } => Some(port),
            _ => None,
        },
    };
    let _ = pidfile::write(&inner.home, &spec.id, &record);
}

async fn run(
    inner: &Arc<Inner>,
    spec: &ServiceSpec,
    stop: &CancellationToken,
    mut adopted: Option<u32>,
) {
    let id = spec.id.as_str();
    let mut tracker = RestartTracker::new(spec.restart.clone());
    // The first launch is what someone asked for. Every launch after a crash is automatic, and a failure of an
    // automatic launch is one more crash, while a failure of the asked-for launch is an `Error`.
    let mut automatic = false;

    loop {
        let mut live = if let Some(pid) = adopted.take() {
            // Already running and ready: straight to watching it.
            Live::Adopted(pid)
        } else {
            let child = match process::spawn(spec, inner.containment.as_ref()) {
                Ok(child) => child,
                Err(error) => {
                    let next = failed_start(inner, id, automatic, &mut tracker, None, error);
                    if carry_on(inner, id, stop, &tracker, next).await {
                        automatic = true;
                        continue;
                    }
                    return;
                }
            };
            let mut live = Live::Own(Box::new(child));
            let pid = live.pid();
            record_started(inner, spec, pid);
            set_pid(inner, id, pid);

            let launch = tokio::select! {
                biased;
                () = stop.cancelled() => Launch::StopRequested,
                code = live.ended(inner.poll) => Launch::Exited(code),
                ready = process::wait_ready(&spec.ready, spec.start_timeout, inner.poll) => {
                    if ready { Launch::Ready } else { Launch::TimedOut }
                }
            };

            let failure = match launch {
                Launch::StopRequested => {
                    stop_flow(inner, spec, &mut live).await;
                    return;
                }
                Launch::Ready => None,
                Launch::Exited(code) => {
                    reaped(inner, id, pid);
                    let what = code.map_or_else(
                        || format!("{id} was ended before it was ready"),
                        |code| format!("{id} exited with code {code} before it was ready"),
                    );
                    Some((code, start_error(id, &what, code)))
                }
                Launch::TimedOut => {
                    live.kill().await;
                    reaped(inner, id, pid);
                    let what = format!(
                        "{id} was not ready within {} s and was stopped",
                        spec.start_timeout.as_secs()
                    );
                    Some((None, start_error(id, &what, None)))
                }
            };
            if let Some((code, error)) = failure {
                let next = failed_start(inner, id, automatic, &mut tracker, code, error);
                if carry_on(inner, id, stop, &tracker, next).await {
                    automatic = true;
                    continue;
                }
                return;
            }
            update(inner, id, Event::Ready, |_| {});
            live
        };

        let pid = live.pid();
        let ended = tokio::select! {
            biased;
            () = stop.cancelled() => Ended::StopRequested,
            code = live.ended(inner.poll) => Ended::Exited(code),
        };
        match ended {
            Ended::StopRequested => {
                stop_flow(inner, spec, &mut live).await;
                return;
            }
            Ended::Exited(code) => {
                reaped(inner, id, pid);
                if spec.restart.recycle_on_exit
                    && update(inner, id, Event::Recycle, |status| {
                        status.pid = None;
                        status.exit_code = code;
                    })
                {
                    // Out by design: straight back in, without a pause and without counting a crash.
                    automatic = true;
                    continue;
                }
                let next = crashed(inner, id, &mut tracker, code, None);
                if carry_on(inner, id, stop, &tracker, next).await {
                    automatic = true;
                    continue;
                }
                return;
            }
        }
    }
}

fn start_error(id: &str, what: &str, exit_code: Option<i32>) -> LunaError {
    LunaError::new(ErrorCode::ServiceStartFailed, what)
        .with_details(json!({ "service": id, "exitCode": exit_code }))
}

fn set_pid(inner: &Inner, id: &str, pid: Option<u32>) {
    let services = lock(&inner.services);
    if let Some(entry) = services.get(id) {
        let mut status = entry.status.borrow().clone();
        status.pid = pid;
        publish(inner, entry, status);
    }
}

/// A launch that did not get the service ready. The asked-for launch ends in `Error` (restarting would not fix
/// a bad configuration); an automatic one counts as another crash.
fn failed_start(
    inner: &Inner,
    id: &str,
    automatic: bool,
    tracker: &mut RestartTracker,
    exit_code: Option<i32>,
    error: LunaError,
) -> Next {
    if automatic {
        return crashed(inner, id, tracker, exit_code, Some(error));
    }
    update(inner, id, Event::StartFailed, |status| {
        status.pid = None;
        status.exit_code = exit_code;
        status.error = Some(error);
    });
    Next::End
}

/// The process died by itself: `Crashed`, and the policy says whether to try again.
fn crashed(
    inner: &Inner,
    id: &str,
    tracker: &mut RestartTracker,
    exit_code: Option<i32>,
    error: Option<LunaError>,
) -> Next {
    let decision = tracker.on_crash(Instant::now());
    update(inner, id, Event::Crash, |status| {
        status.pid = None;
        status.exit_code = exit_code;
        status.error = error;
        status.gave_up = decision == Decision::GiveUp;
    });
    match decision {
        Decision::Restart { after } => Next::Restart(after),
        Decision::GiveUp => Next::End,
    }
}

/// Carry out `next`: wait out the pause and begin the restart (`true`: launch again), or stop here (`false`).
/// A stop that arrives during the pause ends it; the service stays `Crashed` until [`Finish`] turns it into
/// `Stopped`.
async fn carry_on(
    inner: &Inner,
    id: &str,
    stop: &CancellationToken,
    tracker: &RestartTracker,
    next: Next,
) -> bool {
    let Next::Restart(pause) = next else {
        return false;
    };
    tokio::select! {
        biased;
        () = stop.cancelled() => return false,
        () = sleep(pause) => {}
    }
    update(inner, id, Event::Start, |status| {
        status.restarts = tracker.used();
        status.error = None;
        status.gave_up = false;
    });
    true
}

/// Ask the process to leave, force it after the grace period, and record that it is gone.
async fn stop_flow(inner: &Inner, spec: &ServiceSpec, live: &mut Live) {
    update(inner, &spec.id, Event::Stop, |_| {});
    let pid = live.pid();
    live.stop(&spec.stop).await;
    reaped(inner, &spec.id, pid);
    update(inner, &spec.id, Event::Stopped, |status| status.pid = None);
}
