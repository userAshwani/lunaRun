//! The supervisor: owns every service's process and its [`ServiceState`] (docs/SERVICES.md section 2).
//!
//! Each started service gets one **lifecycle task** that launches the process, waits for it to be ready, watches
//! it, restarts it after a crash (the [`RestartPolicy`]) and stops it when asked. That task is the only writer
//! of the service's state while it lives, so there is never a race between "it crashed" and "it was stopped".
//! Every change is applied through the pure table in `state.rs` and announced to a [`ServiceEventSink`].
//!
//! - **Sinks must not call back into the supervisor.** Events are sent while the state lock is held, which keeps
//!   them in order, and before the new status is visible to [`Supervisor::status`] and the `wait_until` family. The
//!   Tauri sink only emits to the webview.
//! - **Sleeping and timing use real time.** Policies are plain values (`RestartPolicy`, `start_timeout`,
//!   `StopPlan::grace`), so tests give them milliseconds.
//! - **Not here yet:** log capture (M1.4 task 3), ports (task 4), the service definitions (task 5), quit order
//!   (task 6) and PID files plus containment (task 2, through [`Containment`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::process::{self, Containment, NoContainment};
use super::restart::{Decision, RestartTracker};
use super::spec::ServiceSpec;
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
    /// How often a ready check is repeated.
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
    pub fn new(handle: Handle, home: LunaHome, sink: impl ServiceEventSink) -> Self {
        Self(Arc::new(Inner {
            handle,
            home,
            sink: Arc::new(sink),
            containment: Arc::new(NoContainment),
            poll: Duration::from_millis(100),
            services: Mutex::new(HashMap::new()),
        }))
    }

    /// Use another [`Containment`] (Job Objects, process groups) for every process started from now on.
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
        let token = CancellationToken::new();
        {
            let mut services = lock(&self.0.services);
            let entry = services.entry(spec.id.clone()).or_insert_with(|| {
                let (status, _) = watch::channel(ServiceStatus::stopped(&spec.id));
                Entry {
                    status,
                    active: None,
                }
            });
            if entry.active.is_some() || entry.status.borrow().state.has_process() {
                return Err(LunaError::new(
                    ErrorCode::Busy,
                    format!(
                        "{} is already {}",
                        spec.id,
                        describe(entry.status.borrow().state)
                    ),
                )
                .with_details(json!({ "service": spec.id })));
            }
            // A fresh run: forget how the last one ended (the state itself is set by the event).
            apply(&self.0, entry, Event::Start, |status| {
                status.pid = None;
                status.restarts = 0;
                status.exit_code = None;
                status.error = None;
                status.gave_up = false;
            });
            entry.active = Some(token.clone());
        }
        let inner = Arc::clone(&self.0);
        self.0.handle.spawn(async move {
            let _finish = Finish {
                inner: Arc::clone(&inner),
                id: spec.id.clone(),
                token: token.clone(),
            };
            run(&inner, &spec, &token).await;
        });
        Ok(())
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

async fn run(inner: &Arc<Inner>, spec: &ServiceSpec, stop: &CancellationToken) {
    let id = spec.id.as_str();
    let mut tracker = RestartTracker::new(spec.restart.clone());
    // The first launch is what someone asked for. Every launch after a crash is automatic, and a failure of an
    // automatic launch is one more crash, while a failure of the asked-for launch is an `Error`.
    let mut automatic = false;

    loop {
        let mut child = match process::spawn(spec, inner.containment.as_ref()) {
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
        set_pid(inner, id, child.id());

        let launch = tokio::select! {
            biased;
            () = stop.cancelled() => Launch::StopRequested,
            status = child.wait() => Launch::Exited(exit_code(status)),
            ready = process::wait_ready(&spec.ready, spec.start_timeout, inner.poll) => {
                if ready { Launch::Ready } else { Launch::TimedOut }
            }
        };

        match launch {
            Launch::StopRequested => {
                stop_flow(inner, spec, &mut child).await;
                return;
            }
            Launch::Exited(code) => {
                let what = code.map_or_else(
                    || format!("{id} was ended before it was ready"),
                    |code| format!("{id} exited with code {code} before it was ready"),
                );
                let error = start_error(id, &what, code);
                let next = failed_start(inner, id, automatic, &mut tracker, code, error);
                if carry_on(inner, id, stop, &tracker, next).await {
                    automatic = true;
                    continue;
                }
                return;
            }
            Launch::TimedOut => {
                process::kill_now(&mut child).await;
                let what = format!(
                    "{id} was not ready within {} s and was stopped",
                    spec.start_timeout.as_secs()
                );
                let error = start_error(id, &what, None);
                let next = failed_start(inner, id, automatic, &mut tracker, None, error);
                if carry_on(inner, id, stop, &tracker, next).await {
                    automatic = true;
                    continue;
                }
                return;
            }
            Launch::Ready => {}
        }

        update(inner, id, Event::Ready, |_| {});
        let ended = tokio::select! {
            biased;
            () = stop.cancelled() => Ended::StopRequested,
            status = child.wait() => Ended::Exited(exit_code(status)),
        };
        match ended {
            Ended::StopRequested => {
                stop_flow(inner, spec, &mut child).await;
                return;
            }
            Ended::Exited(code) => {
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
async fn stop_flow(inner: &Inner, spec: &ServiceSpec, child: &mut tokio::process::Child) {
    update(inner, &spec.id, Event::Stop, |_| {});
    process::stop_child(child, &spec.stop).await;
    update(inner, &spec.id, Event::Stopped, |status| status.pid = None);
}
