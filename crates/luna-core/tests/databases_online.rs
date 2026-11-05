//! Opt-in checks of the database services against the **real MariaDB and MySQL** of this platform. For each engine
//! the newest release with a build for this computer is downloaded by the real installer (checked against the
//! registry's SHA-256), set up for its first start, started through the supervisor, asked a question as `root` with
//! no password over TCP (FR-SV-10), stopped through its own clean shutdown, and started again with its data intact.
//!
//! They download 90 to 330 MB each, run third-party programs from the internet and take a few minutes, so they are
//! ignored by default:
//!
//!     cargo test -p luna-core --test databases_online real_mariadb -- --ignored --nocapture
//!     cargo test -p luna-core --test databases_online real_mysql -- --ignored --nocapture
//!
//! Each proves only the platform it runs on. The data folder of the run is a throw-away one with a space and a
//! non-ASCII letter in its path, which is the real test of the generated `my.ini` (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::registry::Registry;
use luna_core::runtimes::{Installer, list_installed};
use luna_core::services::{
    PortAllocator, ServiceState, ServiceStatus, Supervisor, database_definition, port_owners,
};
use tempfile::TempDir;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

/// Run the client and return what it printed. Fails the test with its output when it fails.
fn sql(client: &Path, port: u16, statement: &str) -> String {
    let output = Command::new(client)
        .args(["--no-defaults", "--protocol=TCP", "-h", "127.0.0.1", "-P"])
        .arg(port.to_string())
        .args([
            "-u",
            "root",
            "--batch",
            "--skip-column-names",
            "-e",
            statement,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "`{statement}` failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

const UMLAUTS: &str = "grüne Äpfel";

/// Upper-case hex of the UTF-8 bytes, as MySQL's `HEX()` prints them.
fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02X}")).collect()
}

fn tail(path: &Path) -> String {
    // Bytes, not `read_to_string`: MySQL writes the path in the ANSI code page, which is not UTF-8.
    fs::read(path).map_or_else(
        |_| "(no log)".to_owned(),
        |bytes| {
            let text = String::from_utf8_lossy(&bytes);
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(25)..].join("\n")
        },
    )
}

async fn real_database(runtime: &str, client_name: &str) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let (os, arch) = (Os::current(), Arch::current());
    let suffix = if os == Os::Windows { ".exe" } else { "" };

    // 1. The newest release with a build for this computer, through the real installer.
    let registry = Registry::parse(include_str!("../../../registry/registry.json")).unwrap();
    let version = registry
        .runtime(runtime)
        .unwrap()
        .releases
        .iter()
        .filter(|release| !release.version.is_prerelease())
        .find(|release| {
            registry
                .artifacts_for(runtime, release.version.as_str(), os, arch)
                .is_ok()
        })
        .map(|release| release.version.to_string())
        .unwrap_or_else(|| panic!("{runtime}: no build for {os:?}/{arch:?}"));
    eprintln!("{runtime} {version}: installing");
    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
    let installer = Installer::new(
        home.clone(),
        Downloader::new(DownloadOptions::default()).unwrap(),
    );
    let started = installer
        .start(&jobs, Arc::new(registry), runtime, &version)
        .unwrap();
    let job = jobs.wait(&started.id).await.unwrap();
    assert_eq!(
        job.state,
        JobState::Succeeded,
        "install failed: {:?}",
        job.error
    );
    let installed = list_installed(&home)
        .unwrap()
        .into_iter()
        .find(|i| i.runtime == runtime)
        .unwrap();

    // 2. The definition: a port (3306 if nothing else has it, which XAMPP often does), the generated config, the spec.
    let ports = PortAllocator::new(home.clone());
    let definition = database_definition(&home, &ports, &installed, os).unwrap();
    let port = definition.port;
    eprintln!(
        "{} on port {port}, config {}",
        definition.spec.id,
        definition.config_file.display()
    );
    eprintln!(
        "--- generated config:\n{}",
        // MySQL's file on Windows is in the ANSI code page, not UTF-8.
        String::from_utf8_lossy(&fs::read(&definition.config_file).unwrap())
    );
    assert!(definition.needs_initialisation());
    definition.spec.validate(&home).unwrap();

    // 3. First start: set the data folder up.
    let begun = Instant::now();
    if let Err(error) = definition
        .initialise(Duration::from_secs(300), &CancellationToken::new())
        .await
    {
        panic!(
            "setup failed: {error}\n--- setup output:\n{}\n--- error log:\n{}",
            tail(&definition.init_log),
            tail(&definition.error_log)
        );
    }
    eprintln!("set up in {:?}", begun.elapsed());
    assert!(!definition.needs_initialisation());
    assert!(
        definition.user_config_file.is_file(),
        "the user's own config file exists"
    );

    // 4. Start it through the supervisor and wait for the real ready check (`admin ping`).
    let states: Arc<std::sync::Mutex<Vec<ServiceStatus>>> = Arc::default();
    let record = Arc::clone(&states);
    let supervisor = Supervisor::new(
        Handle::current(),
        home.clone(),
        move |status: ServiceStatus| {
            record.lock().unwrap().push(status);
        },
    )
    .with_poll_interval(Duration::from_millis(250));
    let begun = Instant::now();
    supervisor.start(definition.spec.clone()).unwrap();
    let status = supervisor
        .wait_until_settled(&definition.spec.id, Duration::from_secs(180))
        .await
        .unwrap();
    assert_eq!(
        status.state,
        ServiceState::Running,
        "did not start: {:?}\n--- error log:\n{}",
        status.error,
        tail(&definition.error_log)
    );
    eprintln!("running after {:?} (pid {:?})", begun.elapsed(), status.pid);
    assert!(
        !port_owners(port).is_empty(),
        "the OS says the database listens on its port"
    );

    // 5. Ask it something, as root with no password, over TCP: what the user's Laravel will do.
    let client = installed
        .dir
        .join("bin")
        .join(format!("{client_name}{suffix}"));
    let answer = sql(&client, port, "SELECT 1, VERSION()");
    eprintln!("answer: {answer}");
    assert!(answer.starts_with('1'), "{answer}");
    sql(
        &client,
        port,
        "CREATE DATABASE lunarun_check CHARACTER SET utf8mb4",
    );
    sql(
        &client,
        port,
        "CREATE TABLE lunarun_check.t (id INT PRIMARY KEY, name VARCHAR(40))",
    );
    // Written and read as hex of its UTF-8 bytes: the client takes and prints text in the console's code page on
    // Windows, so the umlauts would not survive the client, which says nothing about the server.
    sql(
        &client,
        port,
        &format!(
            "INSERT INTO lunarun_check.t VALUES (1, CONVERT(UNHEX('{}') USING utf8mb4))",
            hex(UMLAUTS)
        ),
    );
    // The server uses the settings of the generated config.
    assert_eq!(sql(&client, port, "SELECT @@bind_address"), "127.0.0.1");
    assert_eq!(
        sql(&client, port, "SELECT @@character_set_server"),
        "utf8mb4"
    );
    assert_eq!(sql(&client, port, "SELECT @@port"), port.to_string());

    // 6. Stop it through its own clean shutdown (not a kill), and check the port and the files were released.
    let begun = Instant::now();
    supervisor.stop(&definition.spec.id).await.unwrap();
    eprintln!("stopped in {:?}", begun.elapsed());
    assert_eq!(
        supervisor.status(&definition.spec.id).unwrap().state,
        ServiceState::Stopped
    );
    assert!(
        begun.elapsed() < Duration::from_secs(30),
        "it was killed after the grace period, not shut down"
    );
    let log = String::from_utf8_lossy(&fs::read(&definition.error_log).unwrap()).into_owned();
    assert!(
        log.contains("Shutdown complete"),
        "not a clean shutdown:\n{}",
        tail(&definition.error_log)
    );
    assert!(port_owners(port).is_empty(), "the port is free again");
    assert!(
        !home
            .run_dir()
            .join(format!("{}.pid", definition.spec.id))
            .exists()
    );

    // 7. Start again: the data is there, no setup, and the short timeout applies.
    let again = database_definition(&home, &ports, &installed, os).unwrap();
    assert_eq!(again.port, port, "the same port as before");
    assert!(!again.needs_initialisation());
    again
        .initialise(Duration::from_secs(1), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(again.spec.start_timeout, Duration::from_secs(30));
    supervisor.start(again.spec.clone()).unwrap();
    let status = supervisor
        .wait_until_settled(&again.spec.id, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(
        status.state,
        ServiceState::Running,
        "{:?}\n{}",
        status.error,
        tail(&again.error_log)
    );
    assert_eq!(
        sql(
            &client,
            port,
            "SELECT HEX(name) FROM lunarun_check.t WHERE id = 1"
        ),
        hex(UMLAUTS),
        "the data survived a restart, umlauts included"
    );
    supervisor.stop(&again.spec.id).await.unwrap();
    eprintln!("{runtime} {version}: ok");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 100 MB and runs a real MariaDB"]
async fn real_mariadb_is_set_up_started_queried_stopped_and_started_again() {
    real_database("mariadb", "mariadb").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 300 MB and runs a real MySQL"]
async fn real_mysql_is_set_up_started_queried_stopped_and_started_again() {
    real_database("mysql", "mysql").await;
}
