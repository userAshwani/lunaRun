//! The job event sink must deliver `luna-core` events to the webview under the documented names and
//! with the JSON shape the generated TypeScript types describe. This is an integration test (not a unit
//! test) because Tauri's mock runtime needs the Windows manifest that build.rs only embeds into test
//! targets.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use luna_core::jobs::{JobDone, JobEvent, JobEventSink, JobProgress};
use luna_core::{ErrorCode, LunaError};
use lunarun_lib::events::{JOB_DONE_EVENT, JOB_PROGRESS_EVENT, TauriJobSink};
use tauri::Listener;

/// Collects the raw JSON payloads the webview would receive for `name`.
fn capture(
    app: &tauri::App<tauri::test::MockRuntime>,
    name: &str,
) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let seen: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    app.listen(name, move |event| {
        if let Ok(value) = serde_json::from_str(event.payload())
            && let Ok(mut seen) = sink.lock()
        {
            seen.push(value);
        }
    });
    seen
}

#[test]
fn progress_and_done_reach_the_webview_under_the_documented_event_names() {
    let app = tauri::test::mock_app();
    let progress = capture(&app, JOB_PROGRESS_EVENT);
    let done = capture(&app, JOB_DONE_EVENT);
    let sink = TauriJobSink::new(app.handle().clone());

    sink.emit(JobEvent::Progress(JobProgress {
        job_id: "job-1".into(),
        phase: "download".into(),
        bytes_done: 50,
        bytes_total: Some(100),
        message: None,
    }));
    sink.emit(JobEvent::Done(JobDone {
        job_id: "job-1".into(),
        ok: false,
        error: Some(LunaError::new(ErrorCode::Cancelled, "stopped")),
    }));

    assert_eq!(JOB_PROGRESS_EVENT, "job:progress");
    assert_eq!(JOB_DONE_EVENT, "job:done");
    assert_eq!(
        progress.lock().unwrap().as_slice(),
        [serde_json::json!({
            "jobId": "job-1", "phase": "download", "bytesDone": 50, "bytesTotal": 100, "message": null
        })]
    );
    assert_eq!(
        done.lock().unwrap().as_slice(),
        [serde_json::json!({
            "jobId": "job-1", "ok": false, "error": { "code": "CANCELLED", "message": "stopped" }
        })]
    );
}

#[test]
fn events_for_one_name_do_not_leak_into_the_other() {
    let app = tauri::test::mock_app();
    let progress = capture(&app, JOB_PROGRESS_EVENT);
    let sink = TauriJobSink::new(app.handle().clone());
    sink.emit(JobEvent::Done(JobDone {
        job_id: "job-1".into(),
        ok: true,
        error: None,
    }));
    assert!(progress.lock().unwrap().is_empty());
}
