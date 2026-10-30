//! Background jobs: long tasks such as "install PHP 8.3.14" (download, verify, extract) that the UI starts,
//! watches and can cancel (FR-RT-02, docs/ARCHITECTURE.md section 6).
//!
//! - **Dedupe:** every job has a key (for an install, runtime + version + variant). Starting a job whose
//!   key is already running returns the running job instead of a second one.
//! - **Events:** progress and completion go to a [`JobEventSink`]. luna-core never depends on Tauri; the
//!   app supplies a sink that forwards them to the webview as `job:progress` and `job:done`.
//! - **Throttling:** progress calls can be very frequent (every download chunk). Events are limited to
//!   one per interval per job, but a phase change, completion and the final state are always delivered.
//! - **Cancel:** every job gets a [`CancellationToken`] that the download and extraction engines honour.
//! - **Memory only:** jobs are kept in memory (the newest finished ones for listing) and do not survive a
//!   restart. Installed runtimes are what persist, not the jobs that created them.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::error::{ErrorCode, LunaError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// A snapshot of one job, as listed in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobInfo {
    pub id: String,
    /// What makes two jobs "the same", for example `php@8.3.14`.
    pub key: String,
    /// Title for the UI, for example `Install PHP 8.3.14`.
    pub label: String,
    pub state: JobState,
    /// Machine-readable phase such as `download`, `verify` or `extract`. The UI translates it.
    pub phase: String,
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number | null")]
    pub bytes_total: Option<u64>,
    /// Optional extra detail, such as the file being extracted.
    pub message: Option<String>,
    /// Set when the job failed or was cancelled.
    pub error: Option<LunaError>,
}

/// Payload of the `job:progress` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobProgress {
    pub job_id: String,
    pub phase: String,
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number | null")]
    pub bytes_total: Option<u64>,
    pub message: Option<String>,
}

/// Payload of the `job:done` event. A cancelled job has `ok: false` and the error code `CANCELLED`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobDone {
    pub job_id: String,
    pub ok: bool,
    pub error: Option<LunaError>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Progress(JobProgress),
    Done(JobDone),
}

/// Receives job events. Closures work too: `|event| { ... }`.
pub trait JobEventSink: Send + Sync + 'static {
    fn emit(&self, event: JobEvent);
}

impl<F: Fn(JobEvent) + Send + Sync + 'static> JobEventSink for F {
    fn emit(&self, event: JobEvent) {
        self(event);
    }
}

#[derive(Debug, Clone)]
pub struct JobOptions {
    /// Minimum time between two `job:progress` events of one job.
    pub progress_interval: Duration,
    /// How many finished jobs stay available to [`JobManager::list`].
    pub max_finished: usize,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            progress_interval: Duration::from_millis(100),
            max_finished: 50,
        }
    }
}

/// Result of [`JobManager::start`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedJob {
    pub id: String,
    /// True when a job with the same key was already running, so nothing new was started.
    pub already_running: bool,
}

/// Locks a mutex even if another thread panicked while holding it. The data here is plain state that
/// stays consistent after any single update, so continuing is safe, and a panic inside one job must not
/// take the whole manager down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Throttle {
    last_emit: Option<Instant>,
    last_phase: String,
    /// A progress update was stored but not sent yet.
    pending: bool,
}

struct Shared {
    info: Mutex<JobInfo>,
    cancel: CancellationToken,
    sink: Arc<dyn JobEventSink>,
    throttle: Mutex<Throttle>,
    interval: Duration,
    finished: AtomicBool,
    finished_tx: watch::Sender<bool>,
}

impl Shared {
    fn progress_event(info: &JobInfo) -> JobEvent {
        JobEvent::Progress(JobProgress {
            job_id: info.id.clone(),
            phase: info.phase.clone(),
            bytes_done: info.bytes_done,
            bytes_total: info.bytes_total,
            message: info.message.clone(),
        })
    }
}

/// What a running job uses to report progress and to notice cancellation. Cheap to clone and safe to
/// move into the closures the download and extraction engines take.
#[derive(Clone)]
pub struct JobContext(Arc<Shared>);

impl JobContext {
    pub fn id(&self) -> String {
        lock(&self.0.info).id.clone()
    }

    /// Cancelled by [`JobManager::cancel`]. Pass it to `Downloader::download` and `extract`.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.0.cancel
    }

    /// Report progress. Safe to call as often as you like: events are throttled, state is not.
    pub fn progress(
        &self,
        phase: &str,
        bytes_done: u64,
        bytes_total: Option<u64>,
        message: Option<&str>,
    ) {
        if self.0.finished.load(Ordering::Acquire) {
            return; // a late callback from a worker thread
        }
        let snapshot = {
            let mut info = lock(&self.0.info);
            phase.clone_into(&mut info.phase);
            info.bytes_done = bytes_done;
            info.bytes_total = bytes_total;
            info.message = message.map(str::to_owned);
            Shared::progress_event(&info)
        };

        let due = {
            let now = Instant::now();
            let mut throttle = lock(&self.0.throttle);
            let due = throttle
                .last_emit
                .is_none_or(|last| now.duration_since(last) >= self.0.interval)
                || throttle.last_phase != phase
                || bytes_total == Some(bytes_done);
            if due {
                throttle.last_emit = Some(now);
                phase.clone_into(&mut throttle.last_phase);
                throttle.pending = false;
            } else {
                throttle.pending = true;
            }
            due
        };
        if due {
            self.0.sink.emit(snapshot);
        }
    }

    /// Start a new phase with no byte counts yet.
    pub fn phase(&self, phase: &str, message: Option<&str>) {
        self.progress(phase, 0, None, message);
    }
}

#[derive(Default)]
struct State {
    jobs: HashMap<String, Arc<Shared>>,
    /// Creation order, oldest first.
    order: Vec<String>,
    running_by_key: HashMap<String, String>,
    finished: VecDeque<String>,
}

struct Inner {
    handle: Handle,
    sink: Arc<dyn JobEventSink>,
    options: JobOptions,
    next_id: AtomicU64,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct JobManager {
    inner: Arc<Inner>,
}

impl JobManager {
    /// `handle` is the Tokio runtime that runs jobs. Taking it explicitly means jobs can be started from
    /// synchronous code too, not only from inside the runtime.
    pub fn new(handle: Handle, sink: impl JobEventSink, options: JobOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                handle,
                sink: Arc::new(sink),
                options,
                next_id: AtomicU64::new(1),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Start `work` as a background job, or return the running job with the same `key`.
    pub fn start<F, Fut>(&self, key: &str, label: &str, work: F) -> StartedJob
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let mut state = lock(&self.inner.state);
        if let Some(existing) = state.running_by_key.get(key) {
            return StartedJob {
                id: existing.clone(),
                already_running: true,
            };
        }

        let id = format!("job-{}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (finished_tx, _) = watch::channel(false);
        let info = JobInfo {
            id: id.clone(),
            key: key.to_owned(),
            label: label.to_owned(),
            state: JobState::Running,
            phase: "starting".to_owned(),
            bytes_done: 0,
            bytes_total: None,
            message: None,
            error: None,
        };
        let initial = Shared::progress_event(&info);
        let shared = Arc::new(Shared {
            info: Mutex::new(info),
            cancel: CancellationToken::new(),
            sink: Arc::clone(&self.inner.sink),
            throttle: Mutex::new(Throttle {
                last_emit: Some(Instant::now()),
                last_phase: "starting".to_owned(),
                pending: false,
            }),
            interval: self.inner.options.progress_interval,
            finished: AtomicBool::new(false),
            finished_tx,
        });
        state.jobs.insert(id.clone(), Arc::clone(&shared));
        state.order.push(id.clone());
        state.running_by_key.insert(key.to_owned(), id.clone());
        drop(state);

        self.inner.sink.emit(initial);

        let manager = self.clone();
        let context = JobContext(Arc::clone(&shared));
        let handle = self.inner.handle.clone();
        // The work runs in its own task so a panic inside it is reported as a failed job.
        let worker = handle.spawn(work(context));
        handle.spawn(async move {
            let outcome = match worker.await {
                Ok(result) => result,
                Err(join_error) => Err(LunaError::internal(format!(
                    "the task stopped unexpectedly: {join_error}"
                ))),
            };
            manager.finish(&shared, outcome);
        });

        StartedJob {
            id,
            already_running: false,
        }
    }

    fn finish(&self, shared: &Arc<Shared>, outcome: Result<()>) {
        shared.finished.store(true, Ordering::Release);

        // Send the last stored progress if throttling held it back.
        let pending = {
            let throttle = lock(&shared.throttle);
            throttle
                .pending
                .then(|| Shared::progress_event(&lock(&shared.info)))
        };
        if let Some(event) = pending {
            shared.sink.emit(event);
        }

        let done = {
            let mut info = lock(&shared.info);
            match &outcome {
                Ok(()) => info.state = JobState::Succeeded,
                Err(error) if error.code == ErrorCode::Cancelled => {
                    info.state = JobState::Cancelled
                }
                Err(_) => info.state = JobState::Failed,
            }
            info.error = outcome.err();
            JobDone {
                job_id: info.id.clone(),
                ok: info.state == JobState::Succeeded,
                error: info.error.clone(),
            }
        };

        // Bookkeeping before the event, so a listener that reacts to `Done` by starting the same job
        // again gets a new job, not the one that just finished.
        {
            let mut state = lock(&self.inner.state);
            let (id, key) = {
                let info = lock(&shared.info);
                (info.id.clone(), info.key.clone())
            };
            state.running_by_key.remove(&key);
            state.finished.push_back(id);
            while state.finished.len() > self.inner.options.max_finished {
                if let Some(old) = state.finished.pop_front() {
                    state.jobs.remove(&old);
                    state.order.retain(|id| *id != old);
                }
            }
        }

        shared.sink.emit(JobEvent::Done(done));
        shared.finished_tx.send_replace(true);
    }

    /// Ask a running job to stop. `Ok(true)`: cancellation requested. `Ok(false)`: it had already
    /// finished. `Err(NOT_FOUND)`: no such job (unknown or dropped from the history).
    pub fn cancel(&self, id: &str) -> Result<bool> {
        let shared = lock(&self.inner.state).jobs.get(id).cloned();
        let Some(shared) = shared else {
            return Err(LunaError::new(
                ErrorCode::NotFound,
                format!("there is no job {id:?}"),
            ));
        };
        if shared.finished.load(Ordering::Acquire) {
            return Ok(false);
        }
        shared.cancel.cancel();
        Ok(true)
    }

    pub fn get(&self, id: &str) -> Option<JobInfo> {
        let shared = lock(&self.inner.state).jobs.get(id).cloned()?;
        let info = lock(&shared.info).clone();
        Some(info)
    }

    /// Running jobs first, then finished ones, each newest first.
    pub fn list(&self) -> Vec<JobInfo> {
        let state = lock(&self.inner.state);
        let infos: Vec<JobInfo> = state
            .order
            .iter()
            .rev()
            .filter_map(|id| state.jobs.get(id))
            .map(|shared| lock(&shared.info).clone())
            .collect();
        let (mut running, finished): (Vec<_>, Vec<_>) = infos
            .into_iter()
            .partition(|info| info.state == JobState::Running);
        running.extend(finished);
        running
    }

    /// Wait until the job has finished and return its final state. `None` if the job is unknown.
    pub async fn wait(&self, id: &str) -> Option<JobInfo> {
        let shared = lock(&self.inner.state).jobs.get(id).cloned()?;
        let mut finished = shared.finished_tx.subscribe();
        while !*finished.borrow_and_update() {
            if finished.changed().await.is_err() {
                break;
            }
        }
        let info = lock(&shared.info).clone();
        Some(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A sink that records every event.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<JobEvent>>>);

    impl JobEventSink for Recorder {
        fn emit(&self, event: JobEvent) {
            lock(&self.0).push(event);
        }
    }

    impl Recorder {
        fn events(&self) -> Vec<JobEvent> {
            lock(&self.0).clone()
        }
        fn progress(&self) -> Vec<JobProgress> {
            self.events()
                .into_iter()
                .filter_map(|e| {
                    if let JobEvent::Progress(p) = e {
                        Some(p)
                    } else {
                        None
                    }
                })
                .collect()
        }
        fn done(&self) -> Vec<JobDone> {
            self.events()
                .into_iter()
                .filter_map(|e| {
                    if let JobEvent::Done(d) = e {
                        Some(d)
                    } else {
                        None
                    }
                })
                .collect()
        }
    }

    fn manager(recorder: &Recorder, interval: Duration) -> JobManager {
        JobManager::new(
            Handle::current(),
            recorder.clone(),
            JobOptions {
                progress_interval: interval,
                max_finished: 3,
            },
        )
    }

    #[tokio::test]
    async fn a_successful_job_reports_start_progress_and_done() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::ZERO);
        let started = jobs.start("php@8.3", "Install PHP 8.3", |ctx| async move {
            ctx.progress("download", 50, Some(100), None);
            ctx.progress("download", 100, Some(100), Some("done"));
            Ok(())
        });
        assert!(!started.already_running);

        let info = jobs.wait(&started.id).await.unwrap();
        assert_eq!(info.state, JobState::Succeeded);
        assert_eq!((info.bytes_done, info.bytes_total), (100, Some(100)));
        assert_eq!(info.label, "Install PHP 8.3");
        assert_eq!(info.error, None);

        let progress = recorder.progress();
        assert_eq!(progress.first().unwrap().phase, "starting");
        assert_eq!(progress.last().unwrap().bytes_done, 100);
        assert_eq!(
            recorder.done(),
            vec![JobDone {
                job_id: started.id.clone(),
                ok: true,
                error: None
            }]
        );
        assert!(
            matches!(recorder.events().last(), Some(JobEvent::Done(_))),
            "Done is the last event"
        );
    }

    #[tokio::test]
    async fn a_failing_job_carries_the_typed_error() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::ZERO);
        let started = jobs.start("k", "label", |_| async {
            Err(LunaError::new(ErrorCode::ChecksumMismatch, "bad hash"))
        });
        let info = jobs.wait(&started.id).await.unwrap();
        assert_eq!(info.state, JobState::Failed);
        assert_eq!(
            info.error.as_ref().unwrap().code,
            ErrorCode::ChecksumMismatch
        );
        let done = recorder.done();
        assert!(!done[0].ok);
        assert_eq!(done[0].error.as_ref().unwrap().message, "bad hash");
    }

    #[tokio::test]
    async fn cancelling_stops_the_job_and_reports_cancelled() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::ZERO);
        let started = jobs.start("k", "label", |ctx| async move {
            ctx.cancel_token().cancelled().await;
            Err(LunaError::new(ErrorCode::Cancelled, "stopped"))
        });
        assert!(jobs.cancel(&started.id).unwrap());

        let info = jobs.wait(&started.id).await.unwrap();
        assert_eq!(info.state, JobState::Cancelled);
        let done = &recorder.done()[0];
        assert!(!done.ok);
        assert_eq!(done.error.as_ref().unwrap().code, ErrorCode::Cancelled);
    }

    #[tokio::test]
    async fn cancel_distinguishes_unknown_running_and_finished_jobs() {
        let jobs = manager(&Recorder::default(), Duration::ZERO);
        assert_eq!(
            jobs.cancel("job-999").unwrap_err().code,
            ErrorCode::NotFound
        );

        let started = jobs.start("k", "label", |_| async { Ok(()) });
        jobs.wait(&started.id).await;
        assert!(!jobs.cancel(&started.id).unwrap(), "already finished");
    }

    #[tokio::test]
    async fn a_job_that_ignores_cancellation_but_succeeds_is_succeeded() {
        let jobs = manager(&Recorder::default(), Duration::ZERO);
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let started = jobs.start("k", "label", |_| async move {
            let _ = release_rx.await;
            Ok(())
        });
        jobs.cancel(&started.id).unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(
            jobs.wait(&started.id).await.unwrap().state,
            JobState::Succeeded
        );
    }

    #[tokio::test]
    async fn the_same_key_is_deduplicated_while_running_but_not_after() {
        let jobs = manager(&Recorder::default(), Duration::ZERO);
        let runs = Arc::new(AtomicUsize::new(0));
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

        let counter = Arc::clone(&runs);
        let first = jobs.start("php@8.3", "first", move |_| async move {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = release_rx.await;
            Ok(())
        });
        let counter = Arc::clone(&runs);
        let second = jobs.start("php@8.3", "second", move |_| async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let other = jobs.start("php@8.4", "other key", |_| async { Ok(()) });

        assert!(!first.already_running);
        assert!(second.already_running);
        assert_eq!(second.id, first.id, "the running job is returned");
        assert_ne!(other.id, first.id, "a different key is a different job");

        release_tx.send(()).unwrap();
        jobs.wait(&first.id).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the duplicate never ran");

        let third = jobs.start("php@8.3", "again", |_| async { Ok(()) });
        assert!(
            !third.already_running,
            "after it finished the key can be started again"
        );
        assert_ne!(third.id, first.id);
        assert_eq!(
            jobs.list().iter().find(|j| j.id == first.id).unwrap().label,
            "first"
        );
    }

    #[tokio::test]
    async fn a_panicking_job_becomes_a_failed_job() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::ZERO);
        let started = jobs.start("k", "label", |_| async {
            panic!("boom");
            #[allow(unreachable_code)]
            Ok(())
        });
        let info = jobs.wait(&started.id).await.unwrap();
        assert_eq!(info.state, JobState::Failed);
        assert_eq!(info.error.unwrap().code, ErrorCode::Internal);
        assert_eq!(recorder.done().len(), 1);
        // The manager still works afterwards.
        let next = jobs.start("k", "label", |_| async { Ok(()) });
        assert_eq!(
            jobs.wait(&next.id).await.unwrap().state,
            JobState::Succeeded
        );
    }

    #[tokio::test]
    async fn progress_events_are_throttled_but_state_and_the_final_update_are_exact() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::from_secs(3600)); // effectively "never again" after the first
        let started = jobs.start("k", "label", |ctx| async move {
            for done in 0..10_000u64 {
                ctx.progress("download", done, Some(20_000), None);
            }
            // A different phase always goes out, and so does reaching the total.
            ctx.progress("extract", 5, Some(10), None);
            ctx.progress("extract", 9, Some(10), None); // held back by the throttle ...
            Ok(()) // ... and flushed when the job finishes
        });
        let info = jobs.wait(&started.id).await.unwrap();

        assert_eq!(
            (info.phase.as_str(), info.bytes_done),
            ("extract", 9),
            "state is never throttled"
        );
        let progress = recorder.progress();
        assert!(
            progress.len() < 10,
            "10 000 calls produced only {} events",
            progress.len()
        );
        let last = progress.last().unwrap();
        assert_eq!(
            (last.phase.as_str(), last.bytes_done),
            ("extract", 9),
            "the final update was delivered"
        );
        assert!(
            progress
                .iter()
                .any(|p| p.phase == "extract" && p.bytes_done == 5),
            "phase change goes out at once"
        );
    }

    #[tokio::test]
    async fn reaching_the_total_is_sent_immediately() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::from_secs(3600));
        let started = jobs.start("k", "label", |ctx| async move {
            ctx.progress("download", 1, Some(100), None);
            ctx.progress("download", 100, Some(100), None);
            Ok(())
        });
        jobs.wait(&started.id).await;
        assert!(recorder.progress().iter().any(|p| p.bytes_done == 100));
    }

    #[tokio::test]
    async fn list_puts_running_jobs_first_and_keeps_only_the_newest_finished() {
        let jobs = manager(&Recorder::default(), Duration::ZERO); // max_finished = 3
        let mut ids = Vec::new();
        for n in 0..5 {
            let started = jobs.start(&format!("done-{n}"), &format!("done {n}"), |_| async {
                Ok(())
            });
            jobs.wait(&started.id).await;
            ids.push(started.id);
        }
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let running = jobs.start("running", "running", |_| async move {
            let _ = release_rx.await;
            Ok(())
        });

        let listed: Vec<String> = jobs.list().into_iter().map(|j| j.label).collect();
        assert_eq!(
            listed,
            vec!["running", "done 4", "done 3", "done 2"],
            "oldest finished jobs were dropped"
        );
        assert!(jobs.get(&ids[0]).is_none(), "dropped jobs are gone");
        assert_eq!(jobs.cancel(&ids[0]).unwrap_err().code, ErrorCode::NotFound);

        release_tx.send(()).unwrap();
        jobs.wait(&running.id).await;
    }

    #[tokio::test]
    async fn progress_after_the_job_finished_is_ignored() {
        let recorder = Recorder::default();
        let jobs = manager(&recorder, Duration::ZERO);
        let leaked: Arc<Mutex<Option<JobContext>>> = Arc::default();
        let sink = Arc::clone(&leaked);
        let started = jobs.start("k", "label", move |ctx| async move {
            *lock(&sink) = Some(ctx);
            Ok(())
        });
        jobs.wait(&started.id).await;
        let before = recorder.events().len();
        lock(&leaked)
            .as_ref()
            .unwrap()
            .progress("late", 1, None, None);
        assert_eq!(
            recorder.events().len(),
            before,
            "a late callback emits nothing"
        );
    }

    #[test]
    fn events_serialize_with_the_names_the_ui_expects() {
        let progress = JobProgress {
            job_id: "job-1".into(),
            phase: "download".into(),
            bytes_done: 5,
            bytes_total: Some(10),
            message: None,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            serde_json::json!({ "jobId": "job-1", "phase": "download", "bytesDone": 5, "bytesTotal": 10, "message": null })
        );
        let done = JobDone {
            job_id: "job-1".into(),
            ok: false,
            error: Some(LunaError::new(ErrorCode::Cancelled, "x")),
        };
        assert_eq!(
            serde_json::to_value(&done).unwrap(),
            serde_json::json!({ "jobId": "job-1", "ok": false, "error": { "code": "CANCELLED", "message": "x" } })
        );
        assert_eq!(
            serde_json::to_value(JobState::Succeeded).unwrap(),
            serde_json::json!("succeeded")
        );
    }
}
