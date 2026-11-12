//! A stand-in service for the supervisor tests: this test program itself, hard-linked into a data folder whose path
//! has a space and a non-ASCII letter (NFR-07) and started with an argument that makes it run `stub_child`, which
//! behaves as the environment says (listen on a port, crash after a while, stop when a file appears...). No real
//! PHP or MySQL is involved. Shared by `services.rs` and `services_containment.rs`: each test program is its own
//! stand-in, so the module is declared in both.

// Not every test program uses every helper.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use luna_core::LunaHome;
use luna_core::services::{
    HelperCommand, LogBatch, LogConfig, RestartPolicy, ServiceSpec, ServiceState, ServiceStatus,
    Supervisor,
};
use tokio::runtime::Handle;

// ---------------------------------------------------------------------------------------------
// The stand-in service
// ---------------------------------------------------------------------------------------------

/// Runs only when this program was started as the stand-in service (`LUNA_STUB` set). In a normal test run it
/// returns at once.
#[test]
pub fn stub_child() {
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
        "supervise" => supervise(env),
        // A database's setup program: makes the data folder (or half of it, or nothing).
        "init-ok" | "init-fail" | "init-empty" => init(&mode, env),
        other => panic!("unknown stub mode {other}"),
    }
}

pub fn serve(
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
    if let Some(file) = env("LUNA_STUB_CHILD_PIDFILE") {
        start_own_child(&file, env("LUNA_STUB_CHILD_HEARTBEAT"));
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

    print_things(&env);

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
    let mut chatter = 0_u128;
    let born = Instant::now();
    loop {
        // A stand-in left behind by a failed test must not live forever.
        if born.elapsed() > Duration::from_secs(300) {
            std::process::exit(0);
        }
        if let Some(listener) = &listener
            && listener.accept().is_ok()
            && clock.is_none()
        {
            clock = Some(Instant::now());
        }
        if let Some(every) = millis("LUNA_STUB_CHATTER_MS")
            && born.elapsed().as_millis() / u128::from(every.max(1)) > chatter
        {
            chatter += 1;
            println!("tick {chatter}");
        }
        if let Some(heartbeat) = env("LUNA_STUB_HEARTBEAT") {
            beat += 1;
            let _ = fs::write(heartbeat, beat.to_string());
        }
        if let Some(stop_file) = env("LUNA_STUB_STOPFILE")
            && Path::new(&stop_file).exists()
        {
            println!("goodbye");
            // A service that takes its time to finish what it is doing before it leaves.
            if let Some(delay) = millis("LUNA_STUB_LEAVE_DELAY_MS") {
                std::thread::sleep(Duration::from_millis(delay));
            }
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

/// A database setup program that makes the data folder named in `LUNA_STUB_DATADIR`.
fn init(mode: &str, env: impl Fn(&str) -> Option<String>) -> ! {
    if let Some(counter) = env("LUNA_STUB_COUNTER") {
        let mut text = fs::read_to_string(&counter).unwrap_or_default();
        text.push_str(
            "run
",
        );
        fs::write(counter, text).unwrap();
    }
    let data = PathBuf::from(env("LUNA_STUB_DATADIR").unwrap());
    println!();
    match mode {
        "init-ok" => {
            fs::create_dir_all(data.join("mysql")).unwrap();
            fs::write(data.join("mysql").join("user.MYD"), "the user table").unwrap();
            println!("initialised {}", data.display());
            std::process::exit(0);
        }
        "init-fail" => {
            fs::create_dir_all(&data).unwrap();
            fs::write(data.join("partial.file"), "half of a data folder").unwrap();
            eprintln!("boom: cannot create the system tables");
            std::process::exit(3);
        }
        // Succeeds, makes nothing.
        _ => std::process::exit(0),
    }
}

/// What the stand-in prints, as the environment says: numbered lines on stdout and stderr (interleaved in the order
/// written), raw bytes, one very long line, a line without an ending, and a flood.
fn print_things(env: &impl Fn(&str) -> Option<String>) {
    use std::io::Write;

    let count = |name: &str| env(name).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    // The test harness has just printed "test stub_service::stub_child ... " without a line ending: end it, so that
    // the first line printed here starts a line of its own.
    println!();
    let prefix = env("LUNA_STUB_PREFIX").unwrap_or_else(|| "out".to_owned());
    for n in 0..count("LUNA_STUB_OUT_COUNT").max(count("LUNA_STUB_ERR_COUNT")) {
        if n < count("LUNA_STUB_OUT_COUNT") {
            println!("{prefix} {n}");
        }
        if n < count("LUNA_STUB_ERR_COUNT") {
            eprintln!("err {n}");
        }
    }
    if let Some(hex) = env("LUNA_STUB_RAW_HEX") {
        let bytes: Vec<u8> = hex
            .split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect();
        let mut out = std::io::stdout();
        out.write_all(&bytes).unwrap();
        out.flush().unwrap();
    }
    let long = count("LUNA_STUB_LONG");
    if long > 0 {
        println!("{}", "x".repeat(long as usize));
        println!("after long");
    }
    if let Some(text) = env("LUNA_STUB_NO_NEWLINE") {
        let mut out = std::io::stdout();
        write!(out, "{text}").unwrap();
        out.flush().unwrap();
    }
    let flood = count("LUNA_STUB_FLOOD");
    if flood > 0 {
        // One lock and one buffer: a flood, not a trickle.
        let stdout = std::io::stdout();
        let mut out = std::io::BufWriter::new(stdout.lock());
        for n in 0..flood {
            writeln!(out, "flood {n:06} some padding to make the line longer").unwrap();
        }
        out.flush().unwrap();
        if let Some(done) = env("LUNA_STUB_FLOOD_DONE") {
            fs::write(done, "done").unwrap();
        }
    }
}

/// Start a child of this process that listens on nothing and keeps a heartbeat, and write its pid to `file`. It
/// is what a real service's own children are (php-fpm workers): a process that only containment can clean up.
#[allow(
    clippy::zombie_processes,
    reason = "the child is left running on purpose: only containment may clean it up"
)]
fn start_own_child(file: &str, heartbeat: Option<String>) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(STUB_ARGS);
    // The child must not inherit this process's instructions (it would start a child of its own, and so on).
    for (name, _) in std::env::vars() {
        if name.starts_with("LUNA_STUB") {
            command.env_remove(name);
        }
    }
    command
        .env("LUNA_STUB", "serve")
        .env("LUNA_STUB_NO_LISTEN", "1");
    if let Some(heartbeat) = heartbeat {
        command.env("LUNA_STUB_HEARTBEAT", heartbeat);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().unwrap();
    fs::write(file, child.id().to_string()).unwrap();
    // Dropping the handle does not stop the child: it lives until something kills it.
}

/// A stand-in for lunaRun itself: runs a supervisor and one service, reports the service's pid, and waits. The
/// test kills this process hard to see what happens to the service.
fn supervise(env: impl Fn(&str) -> Option<String>) -> ! {
    let home = LunaHome::at(env("LUNA_STUB_HOME").unwrap()).unwrap();
    let program = PathBuf::from(env("LUNA_STUB_PROGRAM").unwrap());
    let port: u16 = env("LUNA_STUB_PORT").unwrap().parse().unwrap();
    let report = env("LUNA_STUB_REPORT").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let supervisor = Supervisor::new(Handle::current(), home, |_: ServiceStatus| {});
        let mut spec = ServiceSpec::new("orphan-test", &program, port);
        spec.args = stub_args();
        let port_text = port.to_string();
        spec.env = env_of(&[
            ("LUNA_STUB", "serve"),
            ("LUNA_STUB_PORT", port_text.as_str()),
        ]);
        supervisor.start(spec).unwrap();
        let status = supervisor
            .wait_until_settled("orphan-test", Duration::from_secs(20))
            .await
            .unwrap();
        fs::write(report, status.pid.unwrap().to_string()).unwrap();
        // The test kills this process long before; the limit only keeps a forgotten one from living forever.
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    std::process::exit(0)
}

// ---------------------------------------------------------------------------------------------
// The bench
// ---------------------------------------------------------------------------------------------

pub struct Shared {
    pub home: LunaHome,
    pub stub: PathBuf,
}

impl Shared {
    /// A data folder at `dir` (emptied first) with the stand-in program linked into it.
    pub fn create(dir: &Path) -> Self {
        let _ = fs::remove_dir_all(dir);
        let home = LunaHome::at(dir).unwrap();
        home.ensure_layout().unwrap();
        let bin = home.runtimes_dir().join("stub").join("1.0").join("bin");
        fs::create_dir_all(&bin).unwrap();
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
        Self { home, stub }
    }

    /// A folder for one test's files, inside the data folder.
    pub fn scratch(&self, test: &str) -> PathBuf {
        let dir = self.home.run_dir().join(test);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A service that listens on `port` until stopped, with the stand-in's other behaviours in `extra`.
    pub fn serving(&self, id: &str, port: u16, extra: &[(&str, &str)]) -> ServiceSpec {
        let mut spec = ServiceSpec::new(id, &self.stub, port);
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

    pub fn helper(&self, mode: &str, extra: &[(&str, &str)]) -> HelperCommand {
        let mut env = vec![("LUNA_STUB", mode)];
        env.extend_from_slice(extra);
        HelperCommand {
            program: self.stub.clone(),
            args: stub_args(),
            env: env_of(&env),
        }
    }

    /// A supervisor with log settings of the test's choosing, a sink that records every batch with the time it
    /// arrived, and the usual state sink.
    pub fn supervisor_with_logs(&self, config: LogConfig) -> (Supervisor, Events, Batches) {
        self.supervisor_with_log_sink(config, |_| {})
    }

    /// Like `supervisor_with_logs`, and `extra` runs for every batch too (a slow reader, for one).
    pub fn supervisor_with_log_sink(
        &self,
        config: LogConfig,
        extra: impl Fn(&LogBatch) + Send + Sync + 'static,
    ) -> (Supervisor, Events, Batches) {
        let events: Events = Arc::default();
        let batches: Batches = Arc::default();
        let (state, record) = (Arc::clone(&events), Arc::clone(&batches));
        let supervisor = Supervisor::new(
            Handle::current(),
            self.home.clone(),
            move |status: ServiceStatus| state.lock().unwrap().push(status),
        )
        .with_poll_interval(Duration::from_millis(30))
        .with_log_config(config)
        .with_log_sink(move |batch: LogBatch| {
            extra(&batch);
            record.lock().unwrap().push((Instant::now(), batch));
        });
        (supervisor, events, batches)
    }

    pub fn supervisor(&self) -> (Supervisor, Events) {
        let events: Events = Arc::default();
        let sink = Arc::clone(&events);
        let supervisor = Supervisor::new(
            Handle::current(),
            self.home.clone(),
            move |status: ServiceStatus| sink.lock().unwrap().push(status),
        )
        .with_poll_interval(Duration::from_millis(30));
        (supervisor, events)
    }
}

/// One data folder for most tests (every service has its own id and files): the stand-in program is linked in once.
pub fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("services lüna home");
        // Links from earlier runs of this program may still be open on Windows: they stay, this run adds its own.
        Shared::create(&dir)
    })
}

/// A data folder of its own, for a test that looks at the whole run folder (PID files) or at a whole supervisor.
pub fn private(test: &str) -> Shared {
    Shared::create(&Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{test} lüna home")))
}

/// The next port to hand out. Ports come from a counter in a quiet range, 20,000 to 29,999: below the range every
/// OS gives to outgoing connections (so a connection another test makes can never take one), and unique by
/// construction (so parallel tests are never given the same port, and no helper ever has to probe a port by binding it,
/// which is a real listener for a moment and would make a test that was handed that port see it "in use").
fn next_port() -> &'static std::sync::atomic::AtomicU32 {
    static NEXT: std::sync::OnceLock<std::sync::atomic::AtomicU32> = std::sync::OnceLock::new();
    // Each test program starts somewhere else in the range, in case two run at once.
    NEXT.get_or_init(|| std::sync::atomic::AtomicU32::new(20_000 + std::process::id() % 400 * 20))
}

/// A port nothing listens on, never the same one twice in this program.
pub fn free_port() -> u16 {
    free_block(1).start().to_owned()
}

/// `size` consecutive ports that nothing listens on, none of them handed out before.
pub fn free_block(size: u16) -> std::ops::RangeInclusive<u16> {
    use std::sync::atomic::Ordering;

    loop {
        let first = next_port().fetch_add(u32::from(size), Ordering::SeqCst);
        let last = first + u32::from(size) - 1;
        if last > 29_999 {
            next_port().store(20_000, Ordering::SeqCst);
            continue;
        }
        let (first, last) = (u16::try_from(first).unwrap(), u16::try_from(last).unwrap());
        // Something else on this machine may have the port: then take the next ones. Judged by the rule the
        // supervisor itself applies before every start (`check_ports`: the bind fails, or the OS lists a clashing
        // listener, such as another user's program on the IPv6 wildcard address), not by a bind alone: a port that
        // passes a bind but not that rule would make `Supervisor::start` refuse it with `PORT_IN_USE` and no owner
        // (seen twice on Linux CI).
        // (`check_ports` binds the port itself: a bind of our own beforehand would still be listening while it looks.)
        if (first..=last).all(|port| luna_core::services::check_ports(&[port]).is_ok()) {
            return first..=last;
        }
    }
}

/// Wait until `port` can be listened on again. When this program drops a listener of its own, the port is free at once,
/// unless another thread of the program forked at that moment: its child holds a copy of the socket until it execs, a few
/// milliseconds later (seen on macOS CI). A start on that port right after the drop has to wait for it.
pub async fn wait_until_free(port: u16) {
    for _ in 0..200 {
        if luna_core::services::check_ports(&[port]).is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("port {port} was still taken five seconds after its listener was dropped");
}

pub fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)).is_ok()
}

pub fn scratch(test: &str) -> PathBuf {
    shared().scratch(test)
}

pub const STUB_ARGS: [&str; 4] = [
    "--exact",
    "stub_service::stub_child",
    "--nocapture",
    "--test-threads=1",
];

pub fn stub_args() -> Vec<String> {
    STUB_ARGS.iter().map(|a| (*a).to_owned()).collect()
}

pub fn env_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

pub fn serving(id: &str, port: u16, extra: &[(&str, &str)]) -> ServiceSpec {
    shared().serving(id, port, extra)
}

pub fn fast_restarts() -> RestartPolicy {
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

pub fn helper(mode: &str, extra: &[(&str, &str)]) -> HelperCommand {
    shared().helper(mode, extra)
}

pub type Events = Arc<Mutex<Vec<ServiceStatus>>>;

/// Log batches with the moment each arrived.
pub type Batches = Arc<Mutex<Vec<(Instant, LogBatch)>>>;

pub fn supervisor() -> (Supervisor, Events) {
    shared().supervisor()
}

/// The states one service went through, in order, with repeats (a pid arriving) squeezed out.
pub fn states(events: &Events, id: &str) -> Vec<ServiceState> {
    let mut seen: Vec<ServiceState> = Vec::new();
    for status in events.lock().unwrap().iter().filter(|s| s.id == id) {
        if seen.last() != Some(&status.state) {
            seen.push(status.state);
        }
    }
    seen
}

pub const WAIT: Duration = Duration::from_secs(20);

pub async fn settled(supervisor: &Supervisor, id: &str) -> ServiceStatus {
    supervisor.wait_until_settled(id, WAIT).await.unwrap()
}

/// Nothing writes to the heartbeat file any more: the process is gone.
pub async fn assert_process_gone(heartbeat: &Path) {
    let read = || fs::read_to_string(heartbeat).unwrap_or_default();
    let before = read();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(before, read(), "the process is still writing its heartbeat");
}

pub fn launches(counter: &Path) -> usize {
    fs::read_to_string(counter).map_or(0, |text| text.lines().count())
}

/// A copy of the stand-in started directly, not through a supervisor: a process that outlived an earlier lunaRun.
/// A thread collects it when it ends (so it does not stay a zombie that still answers "alive") and ends it on
/// request or when this value is dropped.
pub struct Detached {
    pub pid: u32,
    kill: std::sync::mpsc::Sender<()>,
}

impl Detached {
    pub fn kill(&self) {
        let _ = self.kill.send(());
    }
}

impl Drop for Detached {
    fn drop(&mut self) {
        self.kill();
    }
}

pub fn detached(bench: &Shared, port: u16, extra: &[(&str, &str)]) -> Detached {
    use std::sync::mpsc::{RecvTimeoutError, channel};

    let spec = bench.serving("detached", port, extra);
    let mut command = std::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Not while a port is being probed in another test: the child would inherit the probe (D-030).
    let mut child = {
        let _gate = luna_core::services::spawn_gate();
        command.spawn().unwrap()
    };
    let pid = child.id();
    let (kill, wanted) = channel();
    std::thread::spawn(move || {
        loop {
            match wanted.recv_timeout(Duration::from_millis(50)) {
                Err(RecvTimeoutError::Timeout) => {
                    if child.try_wait().ok().flatten().is_some() {
                        return;
                    }
                }
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
            }
        }
    });
    Detached { pid, kill }
}

/// Write a PID file by hand, the way the supervisor does.
pub fn write_pid_file(bench: &Shared, id: &str, pid: u32, exe: &Path, port: Option<u16>) {
    let exe = serde_json::to_string(&exe.display().to_string()).unwrap();
    let port = port.map_or_else(|| "null".to_owned(), |p| p.to_string());
    let text = format!(r#"{{"schema":1,"pid":{pid},"exe":{exe},"startedAt":1,"port":{port}}}"#);
    fs::write(bench.home.run_dir().join(format!("{id}.pid")), text).unwrap();
}

pub fn pid_file(bench: &Shared, id: &str) -> PathBuf {
    bench.home.run_dir().join(format!("{id}.pid"))
}

/// Wait (up to `limit`) until the process with this number is gone.
pub async fn wait_gone(pid: u32, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    while luna_core::services::process_alive(pid) {
        if Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    true
}
