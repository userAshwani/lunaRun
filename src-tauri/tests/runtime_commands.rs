//! The install, uninstall and usage commands, called the way the webview would call them (through managed
//! state) on Tauri's mock runtime. The install and uninstall logic has its own tests in `luna-core`: this
//! checks the wiring, the error codes the page maps to text, and the JSON the page receives.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::{Arc, Mutex};

use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobEvent, JobManager, JobOptions, JobState};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::{InstallReceipt, Installer, Uninstaller, list_installed};
use luna_core::{ErrorCode, LunaHome};
use lunarun_lib::commands;
use tauri::Manager;
use tempfile::TempDir;

/// What the job manager emitted, like the webview would have received it.
type Recorder = Mutex<Vec<JobEvent>>;

struct Bench {
    _tmp: TempDir,
    app: tauri::App<tauri::test::MockRuntime>,
    home: LunaHome,
    events: Arc<Recorder>,
}

/// An app with the same managed state as `lib.rs` builds, in a data folder with a space and a non-ASCII letter.
fn bench() -> Bench {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    let app = tauri::test::mock_app();
    let events = Arc::new(Recorder::default());
    let sink = Arc::clone(&events);

    app.manage(RegistryHolder::with_bundled().unwrap());
    app.manage(Installer::new(
        home.clone(),
        Downloader::new(DownloadOptions::loopback_test()).unwrap(),
    ));
    app.manage(Uninstaller::new(home.clone()));
    app.manage(JobManager::new(
        tauri::async_runtime::handle().inner().clone(),
        move |event| sink.lock().unwrap().push(event),
        JobOptions::default(),
    ));
    app.manage(home.clone());
    Bench {
        _tmp: tmp,
        app,
        home,
        events,
    }
}

impl Bench {
    /// An installed runtime: a folder with a valid receipt.
    fn install_fake(&self, runtime: &str, version: &str) {
        let dir = self.home.runtime_dir(runtime, version).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin").join("program"), "binary").unwrap();
        InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
            .write(&dir)
            .unwrap();
    }

    fn finish(&self, job_id: &str) -> luna_core::jobs::JobInfo {
        let jobs = self.app.state::<JobManager>();
        tauri::async_runtime::block_on(jobs.wait(job_id)).unwrap()
    }
}

#[test]
fn usage_of_an_installed_version_is_empty_for_now_and_serializes_in_camel_case() {
    let bench = bench();
    bench.install_fake("php", "8.3.14");

    let report = commands::runtime_usage("php".into(), "8.3.14".into(), bench.app.state()).unwrap();

    let value = serde_json::to_value(report).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "runningServices": [],
            "boundProjects": [],
            "dataDirs": [],
            "dataShared": false
        })
    );
}

#[test]
fn usage_of_a_version_that_is_not_installed_is_not_found() {
    let bench = bench();
    let error =
        commands::runtime_usage("php".into(), "8.3.14".into(), bench.app.state()).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn uninstall_runs_as_a_job_removes_the_version_and_reports_done() {
    let bench = bench();
    bench.install_fake("php", "8.3.14");
    bench.install_fake("php", "8.2.20");

    let started = commands::runtime_uninstall(
        "php".into(),
        "8.3.14".into(),
        false,
        false,
        bench.app.state(),
        bench.app.state(),
    )
    .unwrap();
    assert!(!started.already_running);
    let job = bench.finish(&started.id);

    assert_eq!(job.state, JobState::Succeeded, "{job:?}");
    assert_eq!(job.key, "php@8.3.14");
    let left: Vec<String> = list_installed(&bench.home)
        .unwrap()
        .into_iter()
        .map(|i| i.version)
        .collect();
    assert_eq!(left, ["8.2.20"], "only the chosen version went");
    assert!(bench.events.lock().unwrap().iter().any(
        |event| matches!(event, JobEvent::Done(done) if done.job_id == started.id && done.ok)
    ));
}

#[test]
fn uninstalling_what_is_not_installed_fails_the_job_with_not_found() {
    let bench = bench();
    let started = commands::runtime_uninstall(
        "php".into(),
        "8.3.14".into(),
        false,
        false,
        bench.app.state(),
        bench.app.state(),
    )
    .unwrap();

    let job = bench.finish(&started.id);

    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error.unwrap().code, ErrorCode::NotFound);
}

#[test]
fn installing_an_unknown_version_fails_the_job_before_anything_is_downloaded() {
    let bench = bench();

    let started = commands::runtime_install(
        "php".into(),
        "0.0.0".into(),
        bench.app.state(),
        bench.app.state(),
        bench.app.state(),
    )
    .unwrap();
    let job = bench.finish(&started.id);

    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.key, "php@0.0.0");
    assert_eq!(job.error.unwrap().code, ErrorCode::NotFound);
    assert!(list_installed(&bench.home).unwrap().is_empty());
}
