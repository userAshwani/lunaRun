//! Forwards `luna-core` events to the webview as Tauri events. No logic here beyond the mapping.

use luna_core::jobs::{JobEvent, JobEventSink};
use luna_core::services::{LogBatch, ServiceStatus, ShutdownProgress};
use tauri::{AppHandle, Emitter, Runtime};

/// Payload: `JobProgress` (see `src/bindings/JobProgress.ts`).
pub const JOB_PROGRESS_EVENT: &str = "job:progress";
/// Payload: `JobDone` (see `src/bindings/JobDone.ts`).
pub const JOB_DONE_EVENT: &str = "job:done";
/// Payload: `RegistryStatus` (see `src/bindings/RegistryStatus.ts`). Sent when a refresh of the version list
/// has ended, so the Runtimes page asks for the new list.
pub const REGISTRY_UPDATED_EVENT: &str = "registry:updated";

/// Payload: `ServiceStatus` (see `src/bindings/ServiceStatus.ts`). Sent on every change of a service's state, so the
/// Services page shows it at once.
pub const SERVICE_STATE_EVENT: &str = "service:state";
/// Payload: `LogBatch` (see `src/bindings/LogBatch.ts`). New lines of a service's output and log files, in batches
/// of at most ten a second per service.
pub const SERVICE_LOG_EVENT: &str = "service:log";
/// Payload: `ShutdownProgress` (see `src/bindings/ShutdownProgress.ts`). Sent while lunaRun stops its services
/// before it quits.
pub const SHUTDOWN_PROGRESS_EVENT: &str = "shutdown:progress";

/// Tell the webview that a service changed state. Like job events, this never fails the caller: if the window is
/// gone nobody is listening.
pub fn emit_service_state<R: Runtime>(app: &AppHandle<R>, status: &ServiceStatus) {
    let _ = app.emit(SERVICE_STATE_EVENT, status);
}

/// Pass on new lines of a service's output.
pub fn emit_service_log<R: Runtime>(app: &AppHandle<R>, batch: &LogBatch) {
    let _ = app.emit(SERVICE_LOG_EVENT, batch);
}

/// Report how far the orderly stop at quit has come.
pub fn emit_shutdown_progress<R: Runtime>(app: &AppHandle<R>, progress: &ShutdownProgress) {
    let _ = app.emit(SHUTDOWN_PROGRESS_EVENT, progress);
}

/// Tell the webview that the version list was refreshed. Like job events, this never fails the caller: if
/// the window is gone nobody is listening.
pub fn emit_registry_updated<R: Runtime>(
    app: &AppHandle<R>,
    status: &luna_core::registry::RegistryStatus,
) {
    let _ = app.emit(REGISTRY_UPDATED_EVENT, status);
}

pub struct TauriJobSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriJobSink<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> JobEventSink for TauriJobSink<R> {
    fn emit(&self, event: JobEvent) {
        // Emitting only fails when the webview is gone (the window was closed). Nobody is listening
        // then, and a job must never fail because of that.
        let _ = match event {
            JobEvent::Progress(progress) => self.app.emit(JOB_PROGRESS_EVENT, progress),
            JobEvent::Done(done) => self.app.emit(JOB_DONE_EVENT, done),
        };
    }
}
