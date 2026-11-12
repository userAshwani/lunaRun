//! The job event sink must deliver `luna-core` events to the webview under the documented names and
//! with the JSON shape the generated TypeScript types describe. This is an integration test (not a unit
//! test) because Tauri's mock runtime needs the Windows manifest that build.rs only embeds into test
//! targets.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use luna_core::jobs::{JobDone, JobEvent, JobEventSink, JobProgress};
use luna_core::registry::{RegistrySourceKind, RegistryStatus};
use luna_core::services::{
    LogBatch, LogLine, ServiceState, ServiceStatus, ShutdownGroup, ShutdownProgress,
};
use luna_core::{ErrorCode, LunaError};
use lunarun_lib::events::{
    JOB_DONE_EVENT, JOB_PROGRESS_EVENT, REGISTRY_UPDATED_EVENT, SERVICE_LOG_EVENT,
    SERVICE_STATE_EVENT, SHUTDOWN_PROGRESS_EVENT, TauriJobSink, emit_registry_updated,
    emit_service_log, emit_service_state, emit_shutdown_progress,
};
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

#[test]
fn a_refreshed_version_list_is_announced_with_its_status() {
    let app = tauri::test::mock_app();
    let updated = capture(&app, REGISTRY_UPDATED_EVENT);

    emit_registry_updated(
        app.handle(),
        &RegistryStatus {
            source: RegistrySourceKind::Cache,
            age_seconds: Some(3600),
            problem: Some(LunaError::new(ErrorCode::Network, "offline")),
            refreshing: false,
        },
    );

    assert_eq!(REGISTRY_UPDATED_EVENT, "registry:updated");
    assert_eq!(
        updated.lock().unwrap().as_slice(),
        [serde_json::json!({
            "source": "cache", "ageSeconds": 3600, "refreshing": false,
            "problem": { "code": "NETWORK", "message": "offline" }
        })]
    );
}

#[test]
fn a_service_changing_state_reaches_the_webview_with_the_shape_the_page_reads() {
    let app = tauri::test::mock_app();
    let seen = capture(&app, SERVICE_STATE_EVENT);

    emit_service_state(
        app.handle(),
        &ServiceStatus {
            state: ServiceState::Crashed,
            pid: None,
            restarts: 3,
            exit_code: Some(1),
            error: Some(LunaError::new(ErrorCode::ServiceStartFailed, "exited")),
            gave_up: true,
            killed: false,
            ..ServiceStatus::stopped("mysql@8.4")
        },
    );

    assert_eq!(SERVICE_STATE_EVENT, "service:state");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [serde_json::json!({
            "id": "mysql@8.4", "state": "crashed", "pid": null, "restarts": 3, "exitCode": 1,
            "error": { "code": "SERVICE_START_FAILED", "message": "exited" },
            "gaveUp": true, "killed": false
        })]
    );
}

#[test]
fn what_a_service_prints_reaches_the_webview_in_batches() {
    let app = tauri::test::mock_app();
    let seen = capture(&app, SERVICE_LOG_EVENT);

    emit_service_log(
        app.handle(),
        &LogBatch {
            id: "caddy".into(),
            lines: vec![LogLine {
                source: "output".into(),
                line: "serving".into(),
                ts: 1_700_000_000_000,
            }],
        },
    );

    assert_eq!(SERVICE_LOG_EVENT, "service:log");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [serde_json::json!({
            "id": "caddy",
            "lines": [{ "source": "output", "line": "serving", "ts": 1_700_000_000_000_u64 }]
        })]
    );
}

#[test]
fn the_progress_of_a_quit_reaches_the_webview() {
    let app = tauri::test::mock_app();
    let seen = capture(&app, SHUTDOWN_PROGRESS_EVENT);

    emit_shutdown_progress(
        app.handle(),
        &ShutdownProgress {
            group: ShutdownGroup::Databases,
            total: 2,
            done: 1,
            remaining: vec!["mysql@8.4".into()],
            finished: Some("mariadb@12.3".into()),
            elapsed_ms: 1500,
        },
    );

    assert_eq!(SHUTDOWN_PROGRESS_EVENT, "shutdown:progress");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [serde_json::json!({
            "group": "databases", "total": 2, "done": 1, "remaining": ["mysql@8.4"],
            "finished": "mariadb@12.3", "elapsedMs": 1500
        })]
    );
}
