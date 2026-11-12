//! The services wired into the app: the commands the Services page calls, the state the app builds, what the uninstaller
//! is told, and the quit sequence, on Tauri's mock runtime with a data folder that has a space and a non-ASCII letter. The
//! services themselves (supervisor, catalog, ordered stop) have their tests in `luna-core`: this checks the wiring and
//! the JSON the page receives.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use luna_core::diagnostics::{Arch, Os};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::runtimes::{InstallReceipt, Uninstaller};
use luna_core::services::{
    Leftover, PortAllocator, ReadyCheck, ServiceController, ServiceSpec, Supervisor,
};
use luna_core::{ErrorCode, LunaHome};
use lunarun_lib::commands;
use lunarun_lib::services::{self, Leftovers, QuitState};
use tauri::Manager;
use tauri::test::MockRuntime;
use tempfile::TempDir;

struct Bench {
    _tmp: TempDir,
    app: tauri::App<MockRuntime>,
    home: LunaHome,
}

/// An app with what `lib.rs` builds for the services, in a data folder with a space and a non-ASCII letter.
fn bench() -> Bench {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let app = tauri::test::mock_app();
    app.manage(home.clone());
    services::manage(app.handle(), &home);
    // The job manager `lib.rs` adds last: starting, stopping and restarting are jobs.
    app.manage(JobManager::new(
        tauri::async_runtime::handle().inner().clone(),
        |_| {},
        JobOptions::default(),
    ));
    Bench {
        _tmp: tmp,
        app,
        home,
    }
}

impl Bench {
    fn install_fake(&self, runtime: &str, version: &str) {
        let dir = self.home.runtime_dir(runtime, version).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin").join("program"), "binary").unwrap();
        InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
            .write(&dir)
            .unwrap();
    }

    fn list(&self) -> serde_json::Value {
        let rows =
            commands::services_list(self.app.state(), self.app.state(), self.app.state()).unwrap();
        serde_json::to_value(rows).unwrap()
    }
}

#[test]
fn the_app_state_has_everything_the_services_need() {
    let bench = bench();

    // None of these panics: each was managed.
    let _ = bench.app.state::<Supervisor>();
    let _ = bench.app.state::<PortAllocator>();
    let _ = bench.app.state::<Uninstaller>();
    let _ = bench.app.state::<Leftovers>();
    let _ = bench.app.state::<ServiceController>();
    let quit = bench.app.state::<QuitState>();
    assert!(!quit.is_quitting() && !quit.is_finished());
}

#[test]
fn with_nothing_installed_the_list_is_empty_and_so_are_the_leftovers() {
    let bench = bench();

    assert_eq!(bench.list(), serde_json::json!([]));
    assert!(
        commands::services_leftovers(bench.app.state())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn installed_runtimes_become_rows_in_the_json_the_page_reads() {
    let bench = bench();
    bench.install_fake("mysql", "8.4.13");
    bench.install_fake("caddy", "2.11.7");

    let rows = bench.list();

    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let caddy = &rows[0];
    assert_eq!(caddy["id"], "caddy");
    assert_eq!(caddy["kind"], "web");
    assert_eq!(caddy["state"], "stopped");
    assert_eq!(caddy["ports"], serde_json::json!([]));
    assert_eq!(
        (caddy["running"].clone(), caddy["total"].clone()),
        (0.into(), 1.into())
    );
    assert_eq!(caddy["needsSetup"], false);
    let mysql = &rows[1];
    assert_eq!(mysql["id"], "mysql@8.4");
    assert_eq!(mysql["kind"], "database");
    assert_eq!(mysql["runtime"], "mysql");
    assert_eq!(mysql["version"], "8.4.13");
    assert_eq!(mysql["line"], "8.4");
    assert_eq!(mysql["needsSetup"], true, "no data folder yet");
    assert_eq!(
        mysql["members"],
        serde_json::json!([{
            "id": "mysql@8.4", "state": "stopped", "pid": null, "restarts": 0, "exitCode": null,
            "error": null, "gaveUp": false, "killed": false
        }])
    );
}

#[test]
fn opening_the_page_changes_nothing_on_disk() {
    let bench = bench();
    bench.install_fake("mysql", "8.4.13");
    bench.install_fake("php", "8.3.14");
    let before = fs::read_dir(bench.home.config_dir()).unwrap().count();

    for _ in 0..3 {
        bench.list();
    }

    assert_eq!(
        fs::read_dir(bench.home.config_dir()).unwrap().count(),
        before
    );
    assert!(!bench.home.config_dir().join("services.json").exists());
    assert_eq!(fs::read_dir(bench.home.data_dir()).unwrap().count(), 0);
}

#[test]
fn what_was_found_at_startup_is_what_the_command_returns() {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let app = tauri::test::mock_app();
    app.manage(Leftovers::new(vec![Leftover {
        id: "mysql@8.4".into(),
        pid: 4242,
        exe: "/runtimes/mysql/8.4.13/bin/mysqld".into(),
        started_at: 1_767_225_600,
        port: Some(3306),
    }]));

    let found = commands::services_leftovers(app.state()).unwrap();

    assert_eq!(
        serde_json::to_value(found).unwrap(),
        serde_json::json!([{
            "id": "mysql@8.4", "pid": 4242, "exe": "/runtimes/mysql/8.4.13/bin/mysqld",
            "startedAt": 1_767_225_600_u64, "port": 3306
        }])
    );
}

#[test]
fn a_stale_pid_file_is_not_a_leftover() {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    fs::create_dir_all(home.run_dir()).unwrap();
    // A process number that is not running, as a crash would leave behind.
    fs::write(
        home.run_dir().join("mysql@8.4.pid"),
        r#"{"schema":1,"pid":4294967,"exe":"/nope/mysqld","startedAt":1767225600,"port":3306}"#,
    )
    .unwrap();
    let app = tauri::test::mock_app();
    app.manage(home.clone());
    services::manage(app.handle(), &home);

    assert!(
        commands::services_leftovers(app.state())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_uninstaller_asks_the_supervisor_what_runs() {
    let bench = bench();
    bench.install_fake("mysql", "8.4.13");

    let report =
        commands::runtime_usage("mysql".into(), "8.4.13".into(), bench.app.state()).unwrap();
    assert!(report.running_services.is_empty(), "nothing runs");

    // An uninstall of a version that is not installed is still `NOT_FOUND`, as before.
    let error =
        commands::runtime_usage("mysql".into(), "9.9.9".into(), bench.app.state()).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

/// A real process that stays up for a while, copied into the data folder (a service may only run from there): `ping`
/// on Windows, `sleep` elsewhere. Returns the program and its arguments.
fn long_running_program(home: &LunaHome) -> (std::path::PathBuf, Vec<String>) {
    let (source, args): (&str, &[&str]) = if cfg!(windows) {
        (r"C:\Windows\System32\ping.exe", &["-n", "60", "127.0.0.1"])
    } else {
        ("/bin/sleep", &["60"])
    };
    let dir = home.runtimes_dir().join("stand-in").join("bin");
    fs::create_dir_all(&dir).unwrap();
    let program = dir.join(std::path::Path::new(source).file_name().unwrap());
    fs::copy(source, &program).unwrap();
    (program, args.iter().map(|a| (*a).to_owned()).collect())
}

#[test]
fn a_running_service_is_reported_to_the_uninstaller_through_the_apps_own_wiring() {
    let bench = bench();
    bench.install_fake("mysql", "8.4.13");
    let supervisor = bench.app.state::<Supervisor>().inner().clone();
    let (program, args) = long_running_program(&bench.home);
    let mut spec = ServiceSpec::new("mysql@8.4", program, 1);
    spec.args = args;
    spec.ready = ReadyCheck::Immediately;
    spec.ports = Vec::new();
    supervisor.start(spec).unwrap();
    tauri::async_runtime::block_on(
        supervisor.wait_until_settled("mysql@8.4", std::time::Duration::from_secs(30)),
    )
    .unwrap();

    let report =
        commands::runtime_usage("mysql".into(), "8.4.13".into(), bench.app.state()).unwrap();
    // The page lists it, too.
    let rows = bench.list();
    let mysql = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "mysql@8.4")
        .unwrap();

    tauri::async_runtime::block_on(supervisor.stop("mysql@8.4")).unwrap();
    assert_eq!(report.running_services, ["mysql@8.4"]);
    assert_eq!(mysql["state"], "running");
    assert_eq!(mysql["running"], 1);
    assert!(
        commands::runtime_usage("mysql".into(), "8.4.13".into(), bench.app.state())
            .unwrap()
            .running_services
            .is_empty(),
        "stopped again"
    );
}

// ---------------------------------------------------------------------------------------------
// Quit
// ---------------------------------------------------------------------------------------------

#[test]
fn only_the_first_quit_request_starts_the_stop() {
    let quit = QuitState::default();

    assert!(!quit.is_quitting());
    assert!(quit.begin(), "the first request starts it");
    assert!(!quit.begin(), "a second one while it runs does nothing");
    assert!(!quit.begin());
    assert!(quit.is_quitting());
    assert!(!quit.is_finished());

    quit.finish();
    assert!(quit.is_finished());
}

#[test]
fn quitting_with_no_service_running_is_immediate_and_marks_the_quit_finished() {
    let bench = bench();
    let progress = std::sync::Arc::new(std::sync::Mutex::new(0_u32));
    let seen = std::sync::Arc::clone(&progress);
    tauri::Listener::listen(
        &bench.app,
        lunarun_lib::events::SHUTDOWN_PROGRESS_EVENT,
        move |_| {
            *seen.lock().unwrap() += 1;
        },
    );

    let report = tauri::async_runtime::block_on(services::run_quit(bench.app.handle()));

    assert!(report.clean.is_empty() && report.killed.is_empty() && report.failed.is_empty());
    assert_eq!(*progress.lock().unwrap(), 0, "nothing to show progress of");
    assert!(bench.app.state::<QuitState>().is_finished());
    // An exit request after that goes straight through.
    assert!(services::on_exit_requested(bench.app.handle(), Some(0)));
}

// ---------------------------------------------------------------------------------------------
// Start, stop, restart
// ---------------------------------------------------------------------------------------------

impl Bench {
    fn finish(&self, job_id: &str) -> luna_core::jobs::JobInfo {
        tauri::async_runtime::block_on(self.app.state::<JobManager>().wait(job_id)).unwrap()
    }
}

#[test]
fn an_unknown_service_is_not_found_by_all_four_commands() {
    let bench = bench();
    let controller = || bench.app.state::<ServiceController>();
    let jobs = || bench.app.state::<JobManager>();

    for result in [
        commands::service_start("nginx".into(), controller(), jobs()),
        commands::service_stop("nginx".into(), controller(), jobs()),
        commands::service_restart("nginx".into(), controller(), jobs()),
        commands::service_port_reassign("nginx".into(), controller(), jobs()),
    ] {
        assert_eq!(result.unwrap_err().code, ErrorCode::NotFound);
    }
}

#[test]
fn starting_a_service_that_is_not_installed_is_a_job_that_fails_with_not_found() {
    let bench = bench();

    let started =
        commands::service_start("mysql@8.4".into(), bench.app.state(), bench.app.state()).unwrap();
    let info = bench.finish(&started.id);

    assert_eq!(info.key, "service:mysql@8.4");
    assert_eq!(info.label, "Start mysql@8.4");
    assert_eq!(info.state, JobState::Failed);
    assert_eq!(info.error.unwrap().code, ErrorCode::NotFound);
}

#[test]
fn stopping_a_service_that_is_not_running_succeeds() {
    let bench = bench();
    bench.install_fake("caddy", "2.11.7");

    let started =
        commands::service_stop("caddy".into(), bench.app.state(), bench.app.state()).unwrap();

    assert_eq!(bench.finish(&started.id).state, JobState::Succeeded);
}

#[test]
fn a_real_definition_is_built_by_the_command_and_a_missing_program_fails_the_job() {
    // The runtimes here are folders with a placeholder file, not programs: the start gets as far as writing the config
    // and refusing the missing program, and says so as the job's error instead of panicking.
    let bench = bench();
    bench.install_fake("caddy", "2.11.7");

    let started =
        commands::service_start("caddy".into(), bench.app.state(), bench.app.state()).unwrap();
    let info = bench.finish(&started.id);

    assert_eq!(info.state, JobState::Failed);
    assert!(
        bench
            .home
            .config_dir()
            .join("caddy")
            .join("Caddyfile")
            .is_file(),
        "the generated Caddyfile was written"
    );
    assert!(info.error.is_some());
}

#[test]
fn nothing_is_started_once_the_quit_has_begun() {
    let bench = bench();
    bench.install_fake("caddy", "2.11.7");
    assert!(bench.app.state::<QuitState>().begin());

    for result in [
        commands::service_start("caddy".into(), bench.app.state(), bench.app.state()),
        commands::service_restart("caddy".into(), bench.app.state(), bench.app.state()),
        commands::service_port_reassign("caddy".into(), bench.app.state(), bench.app.state()),
    ] {
        assert_eq!(result.unwrap_err().code, ErrorCode::Busy);
    }
    // Stopping is still allowed: it is what the quit does.
    assert!(commands::service_stop("caddy".into(), bench.app.state(), bench.app.state()).is_ok());
    assert!(
        bench.app.state::<Supervisor>().list().is_empty(),
        "no process was started"
    );
}
