//! The supervisor against real processes. The stand-in service is this very test program: it is hard-linked
//! (or copied) into a data folder whose path has a space and a non-ASCII letter (NFR-07), and started with an
//! argument that makes it run `stub_child`, which behaves as the environment says (listen on a port, crash
//! after a while, stop when a file appears...). No real PHP or MySQL is involved.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use luna_core::services::{
    Containment, HelperCommand, ReadyCheck, RestartPolicy, ServiceEventSink, ServiceSpec,
    ServiceState, ServiceStatus, StopMethod, Supervisor,
};
use luna_core::{ErrorCode, LunaHome};
use tokio::runtime::Handle;

// ---------------------------------------------------------------------------------------------
// The stand-in service
// ---------------------------------------------------------------------------------------------

/// Runs only when this program was started as the stand-in service (`LUNA_STUB` set). In a normal test run it
/// returns at once.
#[test]
fn stub_child() {
    let Ok(mode) = std::env::var("LUNA_STUB") else {
        return;
    };
    let env = |name: &str| std::env::var(name).ok();
    let code: i32 = env("LUNA_STUB_EXIT_CODE")
        .and_then(|c| c.parse().ok())
        .unwrap_or(1);
    let millis = |name: &str| env(name).and_then(|v| v.parse::<u64>().ok());

    match mode.as_str() {
        "noop" => std::process::exit(0),
        "exit" => std::process::exit(code),
        "touch" => {
            fs::write(env("LUNA_STUB_FILE").unwrap(), "touched").unwrap();
            std::process::exit(0);
        }
        "exists" => std::process::exit(i32::from(
            !Path::new(&env("LUNA_STUB_FILE").unwrap()).exists(),
        )),
        "serve" => serve(env, millis, code),
        other => panic!("unknown stub mode {other}"),
    }
}

fn serve(
    env: impl Fn(&str) -> Option<String>,
    millis: impl Fn(&str) -> Option<u64>,
    code: i32,
) -> ! {
    // Which launch is this? Every launch adds a line to the counter file.
    let launch = match env("LUNA_STUB_COUNTER") {
        Some(path) => {
            let mut text = fs::read_to_string(&path).unwrap_or_default();
            text.push_str("launch\n");
            fs::write(&path, &text).unwrap();
            text.lines().count() as u64
        }
        None => 1,
    };
    // From this launch on, leave at once, before listening: a start that fails.
    if let Some(from) = env("LUNA_STUB_FAIL_FROM").and_then(|n| n.parse::<u64>().ok())
        && launch >= from
    {
        std::process::exit(
            env("LUNA_STUB_FAIL_CODE")
                .and_then(|c| c.parse().ok())
                .unwrap_or(2),
        );
    }
    if let Some(info) = env("LUNA_STUB_INFO") {
        let cwd = std::env::current_dir().unwrap();
        let probe = env("LUNA_STUB_PROBE").unwrap_or_default();
        fs::write(info, format!("cwd={}\nprobe={probe}\n", cwd.display())).unwrap();
    }
    if let Some(delay) = millis("LUNA_STUB_DELAY_MS") {
        std::thread::sleep(Duration::from_millis(delay));
    }
    let listener = match (env("LUNA_STUB_NO_LISTEN"), env("LUNA_STUB_PORT")) {
        (None, Some(port)) => {
            let listener = TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
            listener.set_nonblocking(true).unwrap();
            Some(listener)
        }
        _ => None,
    };
    if let Some(ready) = env("LUNA_STUB_READYFILE") {
        fs::write(ready, "ready").unwrap();
    }

    // Crash (or leave by design) after a while, but only for the first N launches.
    let leave_after = millis("LUNA_STUB_EXIT_AFTER_MS");
    let leaves = env("LUNA_STUB_LEAVE_FIRST")
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    // The clock for "leave after" starts when the supervisor's ready check connects (so a test does not depend on
    // how fast this machine is), or at once for a service that listens on nothing.
    let mut clock = if listener.is_some() {
        None
    } else {
        Some(Instant::now())
    };
    let mut beat = 0_u64;
    loop {
        if let Some(listener) = &listener
            && listener.accept().is_ok()
            && clock.is_none()
        {
            clock = Some(Instant::now());
        }
        if let Some(heartbeat) = env("LUNA_STUB_HEARTBEAT") {
            beat += 1;
            let _ = fs::write(heartbeat, beat.to_string());
        }
        if let Some(stop_file) = env("LUNA_STUB_STOPFILE")
            && Path::new(&stop_file).exists()
        {
            if let Some(marker) = env("LUNA_STUB_MARKER") {
                fs::write(marker, "left cleanly").unwrap();
            }
            std::process::exit(0);
        }
        if let Some(after) = leave_after
            && launch <= leaves
            && clock.is_some_and(|since| since.elapsed() >= Duration::from_millis(after))
        {
            std::process::exit(code);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------------------------
// The bench
// ---------------------------------------------------------------------------------------------

struct Shared {
    home: LunaHome,
    stub: PathBuf,
}

/// One data folder for all tests (every service has its own id and files): the stand-in program is linked in once.
fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("services lüna home");
        let _ = fs::remove_dir_all(&dir);
        let home = LunaHome::at(&dir).unwrap();
        home.ensure_layout().unwrap();
        let bin = home.runtimes_dir().join("stub").join("1.0").join("bin");
        fs::create_dir_all(&bin).unwrap();
        // One link per test run (a program from an earlier run may still be open on Windows).
        for old in fs::read_dir(&bin).unwrap().flatten() {
            let _ = fs::remove_file(old.path());
        }
        let name = format!(
            "stub-{}{}",
            std::process::id(),
            std::env::consts::EXE_SUFFIX
        );
        let stub = bin.join(name);
        let current = std::env::current_exe().unwrap();
        // Same volume as the target folder, so a link is enough and nothing big is copied.
        if fs::hard_link(&current, &stub).is_err() {
            fs::copy(&current, &stub).unwrap();
        }
        Shared { home, stub }
    })
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)).is_ok()
}

/// A folder for one test's files, inside the data folder.
fn scratch(test: &str) -> PathBuf {
    let dir = shared().home.run_dir().join(test);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const STUB_ARGS: [&str; 4] = ["--exact", "stub_child", "--nocapture", "--test-threads=1"];

fn stub_args() -> Vec<String> {
    STUB_ARGS.iter().map(|a| (*a).to_owned()).collect()
}

fn env_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A service that listens on `port` until stopped, with the stand-in's other behaviours in `extra`.
fn serving(id: &str, port: u16, extra: &[(&str, &str)]) -> ServiceSpec {
    let mut spec = ServiceSpec::new(id, &shared().stub, port);
    spec.args = stub_args();
    let port_text = port.to_string();
    let mut env = vec![
        ("LUNA_STUB", "serve"),
        ("LUNA_STUB_PORT", port_text.as_str()),
    ];
    env.extend_from_slice(extra);
    spec.env = env_of(&env);
    spec.start_timeout = Duration::from_secs(15);
    spec.stop.grace = Duration::from_secs(5);
    spec.restart = fast_restarts();
    spec
}

fn fast_restarts() -> RestartPolicy {
    RestartPolicy {
        max_restarts: 3,
        window: Duration::from_secs(60),
        backoff: vec![
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_millis(200),
        ],
        recycle_on_exit: false,
    }
}

fn helper(mode: &str, extra: &[(&str, &str)]) -> HelperCommand {
    let mut env = vec![("LUNA_STUB", mode)];
    env.extend_from_slice(extra);
    HelperCommand {
        program: shared().stub.clone(),
        args: stub_args(),
        env: env_of(&env),
    }
}

type Events = Arc<Mutex<Vec<ServiceStatus>>>;

fn supervisor() -> (Supervisor, Events) {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let supervisor = Supervisor::new(
        Handle::current(),
        shared().home.clone(),
        move |status: ServiceStatus| sink.lock().unwrap().push(status),
    )
    .with_poll_interval(Duration::from_millis(30));
    (supervisor, events)
}

/// The states one service went through, in order, with repeats (a pid arriving) squeezed out.
fn states(events: &Events, id: &str) -> Vec<ServiceState> {
    let mut seen: Vec<ServiceState> = Vec::new();
    for status in events.lock().unwrap().iter().filter(|s| s.id == id) {
        if seen.last() != Some(&status.state) {
            seen.push(status.state);
        }
    }
    seen
}

const WAIT: Duration = Duration::from_secs(20);

async fn settled(supervisor: &Supervisor, id: &str) -> ServiceStatus {
    supervisor.wait_until_settled(id, WAIT).await.unwrap()
}

/// Nothing writes to the heartbeat file any more: the process is gone.
async fn assert_process_gone(heartbeat: &Path) {
    let read = || fs::read_to_string(heartbeat).unwrap_or_default();
    let before = read();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(before, read(), "the process is still writing its heartbeat");
}

fn launches(counter: &Path) -> usize {
    fs::read_to_string(counter).map_or(0, |text| text.lines().count())
}

use ServiceState::{Crashed, Error, Running, Starting, Stopped, Stopping};

// ---------------------------------------------------------------------------------------------
// Start and stop
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_service_starts_becomes_ready_and_stops() {
    let (supervisor, events) = supervisor();
    let port = free_port();
    let dir = scratch("happy");
    let heartbeat = dir.join("heartbeat");
    let spec = serving(
        "caddy-happy",
        port,
        &[("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap())],
    );

    supervisor.start(spec).unwrap();
    assert_eq!(
        supervisor.status("caddy-happy").unwrap().state,
        Starting,
        "at once, before it is ready"
    );

    let running = settled(&supervisor, "caddy-happy").await;
    assert_eq!(running.state, Running);
    assert!(running.pid.is_some());
    assert!(port_open(port), "it really listens");

    supervisor.stop("caddy-happy").await.unwrap();

    let stopped = supervisor.status("caddy-happy").unwrap();
    assert_eq!(stopped.state, Stopped);
    assert_eq!(stopped.pid, None);
    assert!(!port_open(port));
    assert_process_gone(&heartbeat).await;
    assert_eq!(
        states(&events, "caddy-happy"),
        [Starting, Running, Stopping, Stopped]
    );
    // The pid reaches the sink as soon as the process exists, and is there in the running event.
    let seen = events.lock().unwrap();
    assert!(
        seen.iter()
            .any(|s| s.state == Running && s.pid == running.pid)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_is_not_stopped_cannot_be_started_again() {
    let (supervisor, _events) = supervisor();
    let port = free_port();
    supervisor.start(serving("busy", port, &[])).unwrap();

    let error = supervisor.start(serving("busy", port, &[])).unwrap_err();
    assert_eq!(error.code, ErrorCode::Busy);
    settled(&supervisor, "busy").await;
    let error = supervisor.start(serving("busy", port, &[])).unwrap_err();
    assert_eq!(error.code, ErrorCode::Busy);
    assert_eq!(
        supervisor.status("busy").unwrap().state,
        Running,
        "the running one was left alone"
    );

    supervisor.stop("busy").await.unwrap();
    supervisor.start(serving("busy", port, &[])).unwrap();
    assert_eq!(settled(&supervisor, "busy").await.state, Running);
    supervisor.stop("busy").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spec_that_fails_validation_starts_nothing() {
    let (supervisor, events) = supervisor();
    let mut spec = serving("refused", free_port(), &[]);
    spec.program = std::env::current_exe().unwrap(); // the test program in target/, not in the data folder

    let error = supervisor.start(spec).unwrap_err();

    assert_eq!(error.code, ErrorCode::PathOutsideRoot);
    assert!(supervisor.status("refused").is_none());
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn several_versions_run_side_by_side_on_their_own_ports() {
    let (supervisor, _events) = supervisor();
    let (old, new) = (free_port(), free_port());
    supervisor.start(serving("mysql@8.0", old, &[])).unwrap();
    supervisor.start(serving("mysql@8.4", new, &[])).unwrap();

    assert_eq!(settled(&supervisor, "mysql@8.0").await.state, Running);
    assert_eq!(settled(&supervisor, "mysql@8.4").await.state, Running);
    assert!(port_open(old) && port_open(new));
    let ids: Vec<String> = supervisor.list().into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["mysql@8.0", "mysql@8.4"]);

    supervisor.stop("mysql@8.0").await.unwrap();
    assert!(
        !port_open(old) && port_open(new),
        "the other one keeps running"
    );
    assert_eq!(supervisor.status("mysql@8.4").unwrap().state, Running);
    supervisor.stop("mysql@8.4").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_environment_and_the_working_directory_reach_the_process() {
    let (supervisor, _events) = supervisor();
    let dir = scratch("env wörk dir"); // a space and a non-ASCII letter
    let info = dir.join("info.txt");
    let mut spec = serving(
        "envcheck",
        free_port(),
        &[
            ("LUNA_STUB_INFO", info.to_str().unwrap()),
            ("LUNA_STUB_PROBE", "from the spec"),
        ],
    );
    spec.cwd = Some(dir.clone());

    supervisor.start(spec).unwrap();
    settled(&supervisor, "envcheck").await;
    supervisor.stop("envcheck").await.unwrap();

    let text = fs::read_to_string(info).unwrap();
    assert!(text.contains("probe=from the spec"), "{text}");
    let cwd = text.lines().find_map(|l| l.strip_prefix("cwd=")).unwrap();
    assert_eq!(
        fs::canonicalize(cwd).unwrap(),
        fs::canonicalize(&dir).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_unknown_stopped_and_failed_services() {
    let (supervisor, _events) = supervisor();
    assert_eq!(
        supervisor.stop("nothing").await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        supervisor
            .wait_until_settled("nothing", Duration::from_millis(10))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    let port = free_port();
    supervisor.start(serving("twice", port, &[])).unwrap();
    settled(&supervisor, "twice").await;
    supervisor.stop("twice").await.unwrap();
    supervisor.stop("twice").await.unwrap(); // already stopped: nothing to do
    assert_eq!(supervisor.status("twice").unwrap().state, Stopped);

    // A start that failed leaves `Error`; stopping it clears that.
    let mut spec = serving("failed", free_port(), &[]);
    spec.env = env_of(&[("LUNA_STUB", "exit"), ("LUNA_STUB_EXIT_CODE", "3")]);
    supervisor.start(spec).unwrap();
    assert_eq!(settled(&supervisor, "failed").await.state, Error);
    supervisor.stop("failed").await.unwrap();
    assert_eq!(supervisor.status("failed").unwrap().state, Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_stops_the_old_process_and_starts_with_the_new_spec() {
    let (supervisor, _events) = supervisor();
    let (first, second) = (free_port(), free_port());
    supervisor.start(serving("movable", first, &[])).unwrap();
    settled(&supervisor, "movable").await;

    supervisor
        .restart(serving("movable", second, &[]))
        .await
        .unwrap();

    assert_eq!(settled(&supervisor, "movable").await.state, Running);
    assert!(!port_open(first) && port_open(second));

    // A spec that does not validate must not stop what works.
    let mut bad = serving("movable", second, &[]);
    bad.program = std::env::current_exe().unwrap();
    assert_eq!(
        supervisor.restart(bad).await.unwrap_err().code,
        ErrorCode::PathOutsideRoot
    );
    assert_eq!(supervisor.status("movable").unwrap().state, Running);

    // A service that never ran can be "restarted" too: it just starts.
    supervisor
        .restart(serving("brand-new", free_port(), &[]))
        .await
        .unwrap();
    assert_eq!(settled(&supervisor, "brand-new").await.state, Running);
    supervisor.stop("movable").await.unwrap();
    supervisor.stop("brand-new").await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// A start that does not work
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_exits_before_it_is_ready_is_an_error_not_a_restart_loop() {
    let (supervisor, events) = supervisor();
    let mut spec = serving("exits-early", free_port(), &[]);
    spec.env = env_of(&[("LUNA_STUB", "exit"), ("LUNA_STUB_EXIT_CODE", "3")]);

    supervisor.start(spec).unwrap();
    let status = settled(&supervisor, "exits-early").await;

    assert_eq!(status.state, Error);
    assert_eq!(status.exit_code, Some(3));
    assert_eq!(status.pid, None);
    let error = status.error.unwrap();
    assert_eq!(error.code, ErrorCode::ServiceStartFailed);
    assert!(
        error
            .message
            .contains("exited with code 3 before it was ready"),
        "{}",
        error.message
    );
    assert_eq!(error.details.unwrap()["exitCode"], 3);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        states(&events, "exits-early"),
        [Starting, Error],
        "never restarted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_cannot_be_launched_is_an_error() {
    let (supervisor, _events) = supervisor();
    let dir = shared()
        .home
        .runtimes_dir()
        .join("bogus")
        .join("1.0")
        .join("bin");
    fs::create_dir_all(&dir).unwrap();
    let bogus = dir.join(if cfg!(windows) { "bogus.exe" } else { "bogus" });
    fs::write(&bogus, "this is text, not a program").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bogus, fs::Permissions::from_mode(0o755)).unwrap();
    }

    supervisor
        .start(ServiceSpec::new("bogus", bogus, free_port()))
        .unwrap();
    let status = settled(&supervisor, "bogus").await;

    assert_eq!(status.state, Error);
    assert_eq!(status.error.unwrap().code, ErrorCode::ServiceStartFailed);
    assert_eq!(status.pid, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_is_never_ready_is_stopped_after_the_timeout() {
    let (supervisor, events) = supervisor();
    let heartbeat = scratch("never-ready").join("heartbeat");
    let mut spec = serving(
        "never-ready",
        free_port(),
        &[
            ("LUNA_STUB_NO_LISTEN", "1"),
            ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
        ],
    );
    spec.start_timeout = Duration::from_millis(700);

    supervisor.start(spec).unwrap();
    let status = settled(&supervisor, "never-ready").await;

    assert_eq!(status.state, Error);
    assert!(
        status
            .error
            .unwrap()
            .message
            .contains("was not ready within")
    );
    assert_process_gone(&heartbeat).await;
    assert_eq!(states(&events, "never-ready"), [Starting, Error]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ready_check_can_be_a_helper_program() {
    let (supervisor, events) = supervisor();
    let dir = scratch("helper-ready");
    let ready_file = dir.join("ready");
    let mut spec = serving(
        "db-like",
        free_port(),
        &[
            ("LUNA_STUB_NO_LISTEN", "1"),
            ("LUNA_STUB_DELAY_MS", "600"),
            ("LUNA_STUB_READYFILE", ready_file.to_str().unwrap()),
        ],
    );
    spec.ready = ReadyCheck::Command(helper(
        "exists",
        &[("LUNA_STUB_FILE", ready_file.to_str().unwrap())],
    ));

    supervisor.start(spec).unwrap();
    assert_eq!(supervisor.status("db-like").unwrap().state, Starting);
    assert_eq!(settled(&supervisor, "db-like").await.state, Running);

    assert!(
        ready_file.exists(),
        "it was only ready once the helper saw the file"
    );
    assert_eq!(states(&events, "db-like"), [Starting, Running]);
    supervisor.stop("db-like").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_with_nothing_to_probe_is_running_at_once() {
    let (supervisor, _events) = supervisor();
    let mut spec = serving("worker", free_port(), &[("LUNA_STUB_NO_LISTEN", "1")]);
    spec.ready = ReadyCheck::Immediately;

    supervisor.start(spec).unwrap();

    assert_eq!(settled(&supervisor, "worker").await.state, Running);
    supervisor.stop("worker").await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Crashes
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_crashed_service_comes_back_by_itself() {
    let (supervisor, events) = supervisor();
    let port = free_port();
    let counter = scratch("recovers").join("launches");
    // Crashes on its first launch only.
    let spec = serving(
        "recovers",
        port,
        &[
            ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
            ("LUNA_STUB_EXIT_AFTER_MS", "300"),
            ("LUNA_STUB_LEAVE_FIRST", "1"),
            ("LUNA_STUB_EXIT_CODE", "7"),
        ],
    );

    supervisor.start(spec).unwrap();
    let status = supervisor
        .wait_until("recovers", WAIT, |s| s.restarts == 1 && s.state == Running)
        .await
        .unwrap();

    assert_eq!(
        status.exit_code,
        Some(7),
        "how the first process ended is kept"
    );
    assert!(!status.gave_up && status.error.is_none());
    assert_eq!(launches(&counter), 2);
    assert!(port_open(port));
    assert_eq!(
        states(&events, "recovers"),
        [Starting, Running, Crashed, Starting, Running]
    );
    supervisor.stop("recovers").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn after_three_restarts_it_stays_crashed_and_says_so() {
    let (supervisor, events) = supervisor();
    let counter = scratch("gives-up").join("launches");
    // Crashes on every launch, 150 ms after it is ready.
    let spec = serving(
        "gives-up",
        free_port(),
        &[
            ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
            ("LUNA_STUB_EXIT_AFTER_MS", "150"),
        ],
    );

    supervisor.start(spec).unwrap();
    // Running is "settled" too, but only until the next crash: wait for the end of the crash loop.
    let status = supervisor
        .wait_until("gives-up", WAIT, |s| s.gave_up)
        .await
        .unwrap();

    assert_eq!(status.state, Crashed);
    assert!(status.gave_up, "the UI needs this to notify the user");
    assert_eq!(status.restarts, 3);
    assert_eq!(status.exit_code, Some(1));
    assert_eq!(launches(&counter), 4, "the first launch and three restarts");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(launches(&counter), 4, "and no more");
    let cycle = [Starting, Running, Crashed];
    let expected: Vec<ServiceState> = cycle.iter().cycle().take(12).copied().collect();
    assert_eq!(states(&events, "gives-up"), expected);

    // Stopping it clears the crash.
    supervisor.stop("gives-up").await.unwrap();
    assert_eq!(supervisor.status("gives-up").unwrap().state, Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_fails_during_an_automatic_restart_counts_as_a_crash() {
    let (supervisor, events) = supervisor();
    let counter = scratch("restart-fails").join("launches");
    // The first launch works and crashes after 200 ms. Every later launch exits at once, before it is ready.
    let spec = serving(
        "restart-fails",
        free_port(),
        &[
            ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
            ("LUNA_STUB_EXIT_AFTER_MS", "200"),
            ("LUNA_STUB_FAIL_FROM", "2"),
            ("LUNA_STUB_FAIL_CODE", "2"),
        ],
    );

    supervisor.start(spec).unwrap();
    let status = supervisor
        .wait_until("restart-fails", WAIT, |s| s.gave_up)
        .await
        .unwrap();

    // A failed start of the first launch would be an `Error`. Here it is part of a crash loop: it counts against
    // the three restarts, and the loop ends in `Crashed`.
    assert_eq!(status.state, Crashed);
    assert!(status.gave_up);
    assert_eq!(status.restarts, 3);
    assert_eq!(status.exit_code, Some(2));
    assert_eq!(status.error.unwrap().code, ErrorCode::ServiceStartFailed);
    assert_eq!(launches(&counter), 4);
    assert_eq!(
        states(&events, "restart-fails"),
        [
            Starting, Running, Crashed, Starting, Crashed, Starting, Crashed, Starting, Crashed
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_leaves_by_design_is_started_again_without_counting_a_crash() {
    let (supervisor, events) = supervisor();
    let counter = scratch("recycles").join("launches");
    let mut spec = serving(
        "recycles",
        free_port(),
        &[
            ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
            ("LUNA_STUB_EXIT_AFTER_MS", "250"),
            ("LUNA_STUB_LEAVE_FIRST", "3"),
            ("LUNA_STUB_EXIT_CODE", "0"),
        ],
    );
    spec.restart.recycle_on_exit = true;
    spec.restart.max_restarts = 0; // any counted crash would end it at once

    supervisor.start(spec).unwrap();
    supervisor
        .wait_until("recycles", WAIT, |s| {
            launches(&counter) >= 4 && s.state == Running
        })
        .await
        .unwrap();

    let status = supervisor.status("recycles").unwrap();
    assert_eq!(status.restarts, 0, "not counted");
    assert!(!status.gave_up);
    let seen = states(&events, "recycles");
    assert!(
        !seen.contains(&Crashed) && !seen.contains(&Error),
        "{seen:?}"
    );
    assert_eq!(
        seen,
        [
            Starting, Running, Starting, Running, Starting, Running, Starting, Running
        ]
    );
    supervisor.stop("recycles").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_during_the_pause_before_a_restart_ends_it_at_once() {
    let (supervisor, _events) = supervisor();
    let counter = scratch("pause-stop").join("launches");
    let mut spec = serving(
        "pause-stop",
        free_port(),
        &[
            ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
            ("LUNA_STUB_EXIT_AFTER_MS", "150"),
        ],
    );
    spec.restart.backoff = vec![Duration::from_secs(30)]; // a long pause

    supervisor.start(spec).unwrap();
    supervisor
        .wait_until("pause-stop", WAIT, |s| s.state == Crashed)
        .await
        .unwrap();
    let begun = Instant::now();
    supervisor.stop("pause-stop").await.unwrap();

    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "did not sit out the 30 s pause"
    );
    assert_eq!(supervisor.status("pause-stop").unwrap().state, Stopped);
    assert_eq!(launches(&counter), 1, "no restart happened after the stop");
}

// ---------------------------------------------------------------------------------------------
// How a service is stopped
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_helper_can_ask_the_service_to_shut_itself_down() {
    let (supervisor, _events) = supervisor();
    let dir = scratch("graceful");
    let (stop_file, marker) = (dir.join("stop"), dir.join("marker"));
    let mut spec = serving(
        "graceful",
        free_port(),
        &[
            ("LUNA_STUB_STOPFILE", stop_file.to_str().unwrap()),
            ("LUNA_STUB_MARKER", marker.to_str().unwrap()),
        ],
    );
    spec.stop.method = StopMethod::Command(helper(
        "touch",
        &[("LUNA_STUB_FILE", stop_file.to_str().unwrap())],
    ));
    spec.stop.grace = Duration::from_secs(10);
    supervisor.start(spec).unwrap();
    settled(&supervisor, "graceful").await;

    let begun = Instant::now();
    supervisor.stop("graceful").await.unwrap();

    assert!(marker.exists(), "it left on its own, it was not killed");
    assert!(begun.elapsed() < Duration::from_secs(8));
    assert_eq!(supervisor.status("graceful").unwrap().state, Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_ignores_the_request_is_killed_after_the_grace_period() {
    let (supervisor, _events) = supervisor();
    let dir = scratch("stubborn");
    let (marker, heartbeat) = (dir.join("marker"), dir.join("heartbeat"));
    let mut spec = serving(
        "stubborn",
        free_port(),
        &[
            (
                "LUNA_STUB_STOPFILE",
                dir.join("never-created").to_str().unwrap(),
            ),
            ("LUNA_STUB_MARKER", marker.to_str().unwrap()),
            ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
        ],
    );
    // The helper runs and does nothing: the service keeps going.
    spec.stop.method = StopMethod::Command(helper("noop", &[]));
    spec.stop.grace = Duration::from_millis(800);
    supervisor.start(spec).unwrap();
    settled(&supervisor, "stubborn").await;

    let begun = Instant::now();
    supervisor.stop("stubborn").await.unwrap();

    assert!(
        begun.elapsed() >= Duration::from_millis(800),
        "it was given the grace period first"
    );
    assert!(!marker.exists(), "it was killed, it did not leave cleanly");
    assert_eq!(supervisor.status("stubborn").unwrap().state, Stopped);
    assert_process_gone(&heartbeat).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kill_plan_does_not_ask_first() {
    let (supervisor, _events) = supervisor();
    let heartbeat = scratch("kill-plan").join("heartbeat");
    let mut spec = serving(
        "kill-plan",
        free_port(),
        &[("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap())],
    );
    spec.stop.method = StopMethod::Kill;
    supervisor.start(spec).unwrap();
    settled(&supervisor, "kill-plan").await;

    supervisor.stop("kill-plan").await.unwrap();

    assert_process_gone(&heartbeat).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn quit_and_terminate_both_end_the_process() {
    let (supervisor, _events) = supervisor();
    for (id, method) in [
        ("terminate", StopMethod::Terminate),
        ("quit", StopMethod::Quit),
    ] {
        let heartbeat = scratch(id).join("heartbeat");
        let mut spec = serving(
            id,
            free_port(),
            &[("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap())],
        );
        spec.stop.method = method;
        supervisor.start(spec).unwrap();
        settled(&supervisor, id).await;

        supervisor.stop(id).await.unwrap();

        assert_eq!(supervisor.status(id).unwrap().state, Stopped, "{id}");
        assert_process_gone(&heartbeat).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_can_be_stopped_while_it_is_still_starting() {
    let (supervisor, events) = supervisor();
    let heartbeat = scratch("stop-starting").join("heartbeat");
    let spec = serving(
        "stop-starting",
        free_port(),
        &[
            ("LUNA_STUB_NO_LISTEN", "1"),
            ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
        ],
    );
    supervisor.start(spec).unwrap();
    supervisor
        .wait_until("stop-starting", WAIT, |s| {
            s.state == Starting && s.pid.is_some()
        })
        .await
        .unwrap();

    supervisor.stop("stop-starting").await.unwrap();

    assert_eq!(supervisor.status("stop-starting").unwrap().state, Stopped);
    assert_process_gone(&heartbeat).await;
    assert_eq!(
        states(&events, "stop-starting"),
        [Starting, Stopping, Stopped]
    );
}

// ---------------------------------------------------------------------------------------------
// The seam for containment
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Recording {
    prepared: AtomicU32,
    adopted: Mutex<Vec<u32>>,
    refuse: bool,
}

impl Containment for Recording {
    fn before_spawn(&self, _command: &mut tokio::process::Command) {
        self.prepared.fetch_add(1, Ordering::SeqCst);
    }

    fn after_spawn(&self, child: &tokio::process::Child) -> luna_core::Result<()> {
        if self.refuse {
            return Err(luna_core::LunaError::new(
                ErrorCode::ServiceStartFailed,
                "no job object for you",
            ));
        }
        self.adopted.lock().unwrap().push(child.id().unwrap());
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_process_goes_through_the_containment_before_and_after_it_starts() {
    let (supervisor, _events) = supervisor();
    let recording = Arc::new(Recording::default());
    let supervisor = supervisor.with_containment(recording.clone());

    supervisor
        .start(serving("contained", free_port(), &[]))
        .unwrap();
    let status = settled(&supervisor, "contained").await;

    assert_eq!(recording.prepared.load(Ordering::SeqCst), 1);
    assert_eq!(*recording.adopted.lock().unwrap(), [status.pid.unwrap()]);
    supervisor.stop("contained").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_the_containment_refuses_is_not_kept() {
    let (supervisor, _events) = supervisor();
    let supervisor = supervisor.with_containment(Arc::new(Recording {
        refuse: true,
        ..Recording::default()
    }));
    let heartbeat = scratch("refused-by-containment").join("heartbeat");

    supervisor
        .start(serving(
            "uncontained",
            free_port(),
            &[("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap())],
        ))
        .unwrap();
    let status = settled(&supervisor, "uncontained").await;

    assert_eq!(status.state, Error);
    assert!(status.error.unwrap().message.contains("no job object"));
    assert_process_gone(&heartbeat).await;
}

// ---------------------------------------------------------------------------------------------
// The sink
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_sink_can_be_a_type_as_well_as_a_closure() {
    struct Counting(Arc<AtomicU32>);
    impl ServiceEventSink for Counting {
        fn emit(&self, _status: ServiceStatus) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let count = Arc::new(AtomicU32::new(0));
    let supervisor = Supervisor::new(
        Handle::current(),
        shared().home.clone(),
        Counting(Arc::clone(&count)),
    )
    .with_poll_interval(Duration::from_millis(30));

    supervisor
        .start(serving("counted", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "counted").await;
    supervisor.stop("counted").await.unwrap();

    // Starting, the pid, Running, Stopping, Stopped.
    assert!(count.load(Ordering::SeqCst) >= 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiter_woken_by_a_state_finds_the_sink_already_told() {
    let seen: Events = Arc::default();
    let record = Arc::clone(&seen);
    // A slow sink: if the status were published first, the waiter would wake up long before this returns.
    let supervisor = Supervisor::new(
        Handle::current(),
        shared().home.clone(),
        move |status: ServiceStatus| {
            std::thread::sleep(Duration::from_millis(150));
            record.lock().unwrap().push(status);
        },
    )
    .with_poll_interval(Duration::from_millis(30));

    supervisor
        .start(serving("ordered", free_port(), &[]))
        .unwrap();
    supervisor
        .wait_until("ordered", WAIT, |s| s.state == Running)
        .await
        .unwrap();

    assert!(seen.lock().unwrap().iter().any(|s| s.state == Running));
    supervisor.stop("ordered").await.unwrap();
    assert!(seen.lock().unwrap().iter().any(|s| s.state == Stopped));
}
