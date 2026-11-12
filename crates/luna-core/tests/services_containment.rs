//! No orphans (FR-SV-07): children are contained, every running service has a PID file, and services an earlier
//! lunaRun left running are found again and adopted or stopped. Real processes; every data folder has a space and
//! a non-ASCII letter in its path (NFR-07). The stand-in service is in `stub_service/`.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::path::Path;
use std::time::Duration;

use ServiceState::{Crashed, Running, Starting, Stopped};
use luna_core::ErrorCode;
use luna_core::services::{ServiceState, process_alive, process_exe, same_program};
use serde_json::Value;
use stub_service::*;

/// Wait for a file with a number in it (the stand-in writes the pid of its child there).
async fn number_in(file: &Path) -> u32 {
    for _ in 0..1200 {
        if let Some(number) = fs::read_to_string(file)
            .ok()
            .and_then(|t| t.trim().parse().ok())
        {
            return number;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{} never got a number", file.display());
}

const GONE: Duration = Duration::from_secs(8);

// ---------------------------------------------------------------------------------------------
// Looking a process up
// ---------------------------------------------------------------------------------------------

#[test]
fn this_process_can_be_looked_up_and_nonsense_numbers_cannot() {
    let me = std::process::id();
    assert!(process_alive(me));
    let exe = process_exe(me).expect("the OS names our own program");
    assert!(same_program(&exe, &std::env::current_exe().unwrap()));

    // 0 and 1 are never a service, whatever the OS would say about them (macOS names launchd, for one).
    for nonsense in [0, 1, 999_999_999, u32::MAX] {
        assert!(!process_alive(nonsense), "{nonsense}");
        assert_eq!(process_exe(nonsense), None, "{nonsense}");
    }
}

#[test]
fn programs_are_compared_after_links_and_case_are_resolved() {
    let bench = private("same-program");
    assert!(same_program(&bench.stub, &bench.stub));
    assert!(!same_program(&bench.stub, Path::new("nothing/like/it")));
    // A path that does not exist is compared as written.
    assert!(same_program(Path::new("a/b/c"), Path::new("a/b/c")));
    assert!(!same_program(Path::new("a/b/c"), Path::new("a/b/d")));
    #[cfg(windows)]
    {
        let upper = bench.stub.display().to_string().to_uppercase();
        assert!(same_program(&bench.stub, Path::new(&upper)));
    }
    #[cfg(unix)]
    {
        let link = bench.home.run_dir().join("link-to-stub");
        std::os::unix::fs::symlink(&bench.stub, &link).unwrap();
        assert!(same_program(&bench.stub, &link));
        // Two names of one file are one program: the OS may report either of them.
        let hard = bench.home.run_dir().join("hard-link-to-stub");
        fs::hard_link(&bench.stub, &hard).unwrap();
        assert!(same_program(&bench.stub, &hard));
        // A different file with the same content is another program.
        let copy = bench.home.run_dir().join("copy-of-stub");
        fs::copy(&bench.stub, &copy).unwrap();
        assert!(!same_program(&bench.stub, &copy));
    }
}

// ---------------------------------------------------------------------------------------------
// Containment
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn what_a_service_leaves_running_dies_when_the_service_is_stopped() {
    let bench = private("contain-stop");
    let (supervisor, _events) = bench.supervisor();
    let dir = bench.scratch("t");
    let (child_file, child_beat) = (dir.join("child.pid"), dir.join("child.beat"));
    supervisor
        .start(bench.serving(
            "family",
            free_port(),
            &[
                ("LUNA_STUB_CHILD_PIDFILE", child_file.to_str().unwrap()),
                ("LUNA_STUB_CHILD_HEARTBEAT", child_beat.to_str().unwrap()),
            ],
        ))
        .unwrap();
    settled(&supervisor, "family").await;
    let child = number_in(&child_file).await;
    assert!(process_alive(child), "the service's own child runs");

    supervisor.stop("family").await.unwrap();

    assert!(
        wait_gone(child, GONE).await,
        "the child outlived its service"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_service_leaves_running_dies_when_the_service_crashes() {
    let bench = private("contain-crash");
    let (supervisor, _events) = bench.supervisor();
    let dir = bench.scratch("t");
    let child_file = dir.join("child.pid");
    let mut spec = bench.serving(
        "family",
        free_port(),
        &[
            ("LUNA_STUB_CHILD_PIDFILE", child_file.to_str().unwrap()),
            ("LUNA_STUB_EXIT_AFTER_MS", "200"),
        ],
    );
    spec.restart.max_restarts = 0; // the first crash is the last
    supervisor.start(spec).unwrap();
    let child = number_in(&child_file).await;

    supervisor
        .wait_until("family", WAIT, |status| status.gave_up)
        .await
        .unwrap();

    assert!(
        wait_gone(child, GONE).await,
        "the child outlived its crashed service"
    );
}

/// The part of "no orphans" that only a real crash of lunaRun can show. The stand-in runs a supervisor and one
/// service; the test kills the stand-in hard (no cleanup code runs) and looks at the service.
#[tokio::test(flavor = "multi_thread")]
async fn when_lunarun_dies_hard_the_service_is_killed_on_windows_and_found_again_elsewhere() {
    let bench = private("hard-kill");
    let port = free_port();
    let report = bench.scratch("t").join("report");
    // Not while a port is being probed in another test: the child would inherit the probe (D-030).
    let gate = luna_core::services::spawn_gate();
    let mut lunarun = std::process::Command::new(&bench.stub)
        .args(stub_args())
        .env("LUNA_STUB", "supervise")
        .env("LUNA_STUB_HOME", bench.home.root())
        .env("LUNA_STUB_PROGRAM", &bench.stub)
        .env("LUNA_STUB_PORT", port.to_string())
        .env("LUNA_STUB_REPORT", &report)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    drop(gate);
    let service = number_in(&report).await;
    assert!(process_alive(service));
    assert!(
        pid_file(&bench, "orphan-test").exists(),
        "recorded while it runs"
    );

    lunarun.kill().unwrap();
    lunarun.wait().unwrap();

    let (supervisor, _events) = bench.supervisor();
    if cfg!(windows) {
        // The Job Object was closed with the process that owned it, and closing it kills what is inside.
        assert!(
            wait_gone(service, GONE).await,
            "the service outlived lunaRun"
        );
        assert!(supervisor.scan_leftovers().is_empty());
        assert!(
            !pid_file(&bench, "orphan-test").exists(),
            "the stale file is cleaned up"
        );
    } else {
        // A process group does not die with its parent: the service is still there, and the PID file finds it.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(process_alive(service));
        let found = supervisor.scan_leftovers();
        assert_eq!(
            found.len(),
            1,
            "found {found:?}; the service is alive: {}, the OS says it runs {:?}",
            process_alive(service),
            process_exe(service)
        );
        assert_eq!(
            (found[0].id.as_str(), found[0].pid, found[0].port),
            ("orphan-test", service, Some(port))
        );
        supervisor.stop_leftover("orphan-test").await.unwrap();
        assert!(wait_gone(service, GONE).await);
        assert!(!pid_file(&bench, "orphan-test").exists());
    }
}

// ---------------------------------------------------------------------------------------------
// PID files
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_running_service_has_a_pid_file_that_is_gone_when_it_stops() {
    let bench = private("pidfile-lifecycle");
    let (supervisor, _events) = bench.supervisor();
    let port = free_port();
    supervisor.start(bench.serving("db", port, &[])).unwrap();
    let running = settled(&supervisor, "db").await;

    let file = pid_file(&bench, "db");
    let record: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(record["schema"], 1);
    assert_eq!(record["pid"], running.pid.unwrap());
    assert_eq!(record["port"], port);
    assert!(
        record["exe"]
            .as_str()
            .unwrap()
            .ends_with(bench.stub.file_name().unwrap().to_str().unwrap())
    );
    assert!(
        record["startedAt"].as_u64().unwrap() > 1_700_000_000,
        "seconds since 1970"
    );

    supervisor.stop("db").await.unwrap();
    assert!(!file.exists(), "gone as soon as the service is stopped");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_pid_file_is_left_by_a_start_that_failed_or_a_service_that_crashed() {
    let bench = private("pidfile-failures");
    let (supervisor, _events) = bench.supervisor();

    let mut early = bench.serving("early", free_port(), &[]);
    early.env = env_of(&[("LUNA_STUB", "exit"), ("LUNA_STUB_EXIT_CODE", "3")]);
    supervisor.start(early).unwrap();
    settled(&supervisor, "early").await;
    assert!(
        !pid_file(&bench, "early").exists(),
        "exited before it was ready"
    );

    let mut slow = bench.serving("slow", free_port(), &[("LUNA_STUB_NO_LISTEN", "1")]);
    slow.start_timeout = Duration::from_millis(500);
    supervisor.start(slow).unwrap();
    settled(&supervisor, "slow").await;
    assert!(!pid_file(&bench, "slow").exists(), "never ready");

    let mut crashing = bench.serving(
        "crashing",
        free_port(),
        &[("LUNA_STUB_EXIT_AFTER_MS", "150")],
    );
    crashing.restart.max_restarts = 1;
    supervisor.start(crashing).unwrap();
    supervisor
        .wait_until("crashing", WAIT, |status| status.gave_up)
        .await
        .unwrap();
    assert!(!pid_file(&bench, "crashing").exists(), "crashed for good");

    assert!(
        !fs::read_dir(bench.home.run_dir())
            .unwrap()
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "pid"))
    );
}

// ---------------------------------------------------------------------------------------------
// Finding what an earlier lunaRun left
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_scan_returns_what_still_runs_and_cleans_up_everything_else() {
    let bench = private("scan");
    let (supervisor, _events) = bench.supervisor();
    let port = free_port();
    let alive = detached(&bench, port, &[]);
    write_pid_file(&bench, "alive", alive.pid, &bench.stub, Some(port));
    // The process is gone.
    write_pid_file(&bench, "dead", 999_999_999, &bench.stub, None);
    // The number is in use, by another program than the one recorded (the number was reused).
    let elsewhere = bench.home.runtimes_dir().join("not").join("it.exe");
    write_pid_file(&bench, "reused", std::process::id(), &elsewhere, None);
    // Unreadable, and not a record at all.
    fs::write(pid_file(&bench, "corrupt"), "this is not json").unwrap();
    write_pid_file(&bench, "zero", 0, &bench.stub, None);
    // Written by a newer lunaRun: not understood, so left exactly as it is.
    let newer = r#"{"schema":99,"something":"new"}"#;
    fs::write(pid_file(&bench, "newer"), newer).unwrap();
    // Run by this supervisor: not a leftover.
    supervisor
        .start(bench.serving("mine", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "mine").await;

    let found = supervisor.scan_leftovers();

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].id, "alive");
    assert_eq!(found[0].pid, alive.pid);
    assert_eq!(found[0].port, Some(port));
    assert_eq!(found[0].exe, bench.stub.display().to_string());
    assert_eq!(found[0].started_at, 1);
    for gone in ["dead", "reused", "corrupt", "zero"] {
        assert!(
            !pid_file(&bench, gone).exists(),
            "{gone} is stale and should be deleted"
        );
    }
    assert!(pid_file(&bench, "alive").exists());
    assert_eq!(
        fs::read_to_string(pid_file(&bench, "newer")).unwrap(),
        newer
    );
    assert!(
        pid_file(&bench, "mine").exists(),
        "a running service keeps its file"
    );
    assert!(
        process_alive(std::process::id()),
        "the reused number was not touched"
    );
    // What is left in the run folder: the live ones and the newer file, and no copy set aside of the damaged one.
    let mut files: Vec<String> = fs::read_dir(bench.home.run_dir())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["alive.pid", "mine.pid", "newer.pid"]);
    supervisor.stop("mine").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_leftover_can_be_stopped() {
    let bench = private("stop-leftover");
    let (supervisor, _events) = bench.supervisor();
    let port = free_port();
    let alive = detached(&bench, port, &[]);
    write_pid_file(&bench, "db", alive.pid, &bench.stub, Some(port));
    assert!(supervisor.scan_leftovers().iter().any(|l| l.id == "db"));

    supervisor.stop_leftover("db").await.unwrap();

    assert!(wait_gone(alive.pid, GONE).await);
    assert!(!pid_file(&bench, "db").exists());
    let again = supervisor.stop_leftover("db").await.unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_leftover_never_touches_a_process_that_only_has_the_same_number() {
    let bench = private("stop-reused");
    let (supervisor, _events) = bench.supervisor();
    let elsewhere = bench.home.runtimes_dir().join("not").join("it.exe");
    write_pid_file(&bench, "db", std::process::id(), &elsewhere, None);

    let error = supervisor.stop_leftover("db").await.unwrap_err();

    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(
        process_alive(std::process::id()),
        "this test is still running, so it was not killed"
    );
    assert_eq!(
        supervisor
            .stop_leftover("nothing-at-all")
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

// ---------------------------------------------------------------------------------------------
// Adopting
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_adopted_service_is_tracked_like_one_that_was_started_here() {
    let bench = private("adopt");
    let (supervisor, events) = bench.supervisor();
    let port = free_port();
    let alive = detached(&bench, port, &[]);
    write_pid_file(&bench, "db", alive.pid, &bench.stub, Some(port));

    supervisor
        .adopt(bench.serving("db", port, &[]))
        .await
        .unwrap();

    let status = supervisor.status("db").unwrap();
    assert_eq!(status.state, Running);
    assert_eq!(status.pid, Some(alive.pid));
    assert!(process_alive(alive.pid), "adopting does not restart it");
    assert!(
        supervisor.scan_leftovers().is_empty(),
        "no longer a leftover"
    );
    assert_eq!(
        supervisor
            .start(bench.serving("db", port, &[]))
            .unwrap_err()
            .code,
        ErrorCode::Busy,
        "it is running, as far as the supervisor is concerned"
    );

    supervisor.stop("db").await.unwrap();

    assert_eq!(supervisor.status("db").unwrap().state, Stopped);
    assert!(wait_gone(alive.pid, GONE).await);
    assert!(!pid_file(&bench, "db").exists());
    assert_eq!(
        states(&events, "db"),
        [Starting, Running, ServiceState::Stopping, Stopped]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_adopted_that_is_not_there_is_another_program_or_does_not_answer() {
    let bench = private("adopt-refused");
    let (supervisor, _events) = bench.supervisor();
    let port = free_port();
    let alive = detached(&bench, port, &[]);
    write_pid_file(&bench, "db", alive.pid, &bench.stub, Some(port));

    // Not running at all.
    write_pid_file(&bench, "ghost", 999_999_999, &bench.stub, None);
    assert_eq!(
        supervisor
            .adopt(bench.serving("ghost", free_port(), &[]))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    // Running, but the spec names another program than the one that runs.
    let other = bench
        .stub
        .with_file_name(format!("other{}", std::env::consts::EXE_SUFFIX));
    fs::copy(&bench.stub, &other).unwrap();
    let mut wrong = bench.serving("db", port, &[]);
    wrong.program = other;
    let error = supervisor.adopt(wrong).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidPath);

    // Running, but it does not answer where the spec says it should.
    let mut silent = bench.serving("db", free_port(), &[]);
    silent.start_timeout = Duration::from_millis(400);
    let error = supervisor.adopt(silent).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceStartFailed);

    assert!(
        process_alive(alive.pid),
        "a refused adoption leaves the process alone"
    );
    assert!(pid_file(&bench, "db").exists());
    assert!(supervisor.status("db").is_none(), "and creates no service");

    // A service of this supervisor cannot be adopted.
    supervisor
        .start(bench.serving("mine", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "mine").await;
    assert_eq!(
        supervisor
            .adopt(bench.serving("mine", free_port(), &[]))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    supervisor.stop("mine").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_adopted_service_that_dies_is_restarted_by_the_policy() {
    let bench = private("adopt-crash");
    let (supervisor, events) = bench.supervisor();
    let port = free_port();
    let alive = detached(&bench, port, &[]);
    write_pid_file(&bench, "db", alive.pid, &bench.stub, Some(port));
    supervisor
        .adopt(bench.serving("db", port, &[]))
        .await
        .unwrap();
    let old = alive.pid;

    alive.kill();
    let status = supervisor
        .wait_until("db", WAIT, |s| s.restarts == 1 && s.state == Running)
        .await
        .unwrap();

    assert_ne!(
        status.pid,
        Some(old),
        "a new process, started by the supervisor"
    );
    assert!(process_alive(status.pid.unwrap()));
    assert_eq!(
        states(&events, "db"),
        [Starting, Running, Crashed, Starting, Running]
    );
    assert!(
        pid_file(&bench, "db").exists(),
        "the new process is recorded"
    );

    supervisor.stop("db").await.unwrap();
    assert!(!pid_file(&bench, "db").exists());
}
