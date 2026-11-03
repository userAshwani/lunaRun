//! Forwards `luna-core` events to the webview as Tauri events. No logic here beyond the mapping.

use luna_core::jobs::{JobEvent, JobEventSink};
use tauri::{AppHandle, Emitter, Runtime};

/// Payload: `JobProgress` (see `src/bindings/JobProgress.ts`).
pub const JOB_PROGRESS_EVENT: &str = "job:progress";
/// Payload: `JobDone` (see `src/bindings/JobDone.ts`).
pub const JOB_DONE_EVENT: &str = "job:done";
/// Payload: `RegistryStatus` (see `src/bindings/RegistryStatus.ts`). Sent when a refresh of the version list
/// has ended, so the Runtimes page asks for the new list.
pub const REGISTRY_UPDATED_EVENT: &str = "registry:updated";

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
