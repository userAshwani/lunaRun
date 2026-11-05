//! The database services: the spec and the generated config built from an installed runtime, and the first-start setup
//! (FR-SV-02, FR-SV-08 to FR-SV-10, D-027). The runtimes here are folders with stand-in program files; the real
//! MariaDB and MySQL are run by the ignored tests in `databases_online.rs`. Every data folder has a space and a
//! non-ASCII letter in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use luna_core::diagnostics::{Arch, Os};
use luna_core::runtimes::{InstallReceipt, InstalledRuntime};
use luna_core::services::{
    DatabaseDefinition, Engine, PortAllocator, PortRanges, ReadyCheck, StopMethod,
    database_definition,
};
use luna_core::{ErrorCode, LunaHome};
use stub_service::*;
use tokio_util::sync::CancellationToken;

/// Ranges of ports of our own, so the tests do not depend on what runs on this machine.
fn own_ranges() -> PortRanges {
    PortRanges {
        database: free_block(8),
        ..PortRanges::default()
    }
}

fn allocator(home: &LunaHome) -> PortAllocator {
    PortAllocator::new(home.clone()).with_ranges(own_ranges())
}

fn program(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "stand-in").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// An installed database: its folder with the program files of the layout for `os`, and a receipt.
fn installed(home: &LunaHome, runtime: &str, version: &str, os: Os) -> InstalledRuntime {
    let dir = home.runtime_dir(runtime, version).unwrap();
    let suffix = if os == Os::Windows { ".exe" } else { "" };
    let names: &[&str] = match runtime {
        "mysql" => &["mysqld", "mysqladmin", "mysql"],
        _ => &["mariadbd", "mariadb-admin", "mariadb-install-db", "mariadb"],
    };
    for name in names {
        program(&dir.join("bin").join(format!("{name}{suffix}")));
    }
    InstalledRuntime {
        runtime: runtime.to_owned(),
        version: version.to_owned(),
        dir,
        receipt: InstallReceipt::new(runtime, version, os, Arch::X86_64, false, &[]),
    }
}

fn define(
    home: &LunaHome,
    ports: &PortAllocator,
    runtime: &InstalledRuntime,
    os: Os,
) -> DatabaseDefinition {
    database_definition(home, ports, runtime, os).unwrap()
}

fn helper_args(definition: &DatabaseDefinition) -> (PathBuf, Vec<String>, Vec<String>) {
    let ReadyCheck::Command(ready) = &definition.spec.ready else {
        panic!("the ready check is a helper program");
    };
    let StopMethod::Command(stop) = &definition.spec.stop.method else {
        panic!("the stop is a helper program");
    };
    (ready.program.clone(), ready.args.clone(), stop.args.clone())
}

const PLATFORMS: [Os; 3] = [Os::Windows, Os::Linux, Os::Macos];

// ---------------------------------------------------------------------------------------------
// The definition
// ---------------------------------------------------------------------------------------------

/// A generated config as text. MySQL's file on Windows is in the ANSI code page, and the test folders have an `ü`
/// in their path, so the file is not valid UTF-8 there: decode lossily (the settings under test are ASCII).
fn read_text(path: &Path) -> String {
    String::from_utf8_lossy(&fs::read(path).unwrap()).into_owned()
}

#[test]
fn a_definition_has_every_part_for_every_engine_and_platform() {
    for (runtime, version, engine, label) in [
        ("mysql", "8.4.13", Engine::Mysql, "MySQL"),
        ("mariadb", "11.8.9", Engine::Mariadb, "MariaDB"),
    ] {
        for os in PLATFORMS {
            let bench = private(&format!("db-def-{runtime}-{os:?}"));
            let ports = allocator(&bench.home);
            let installed = installed(&bench.home, runtime, version, os);
            let line = if runtime == "mysql" { "8.4" } else { "11.8" };

            let d = define(&bench.home, &ports, &installed, os);
            let spec = &d.spec;
            let suffix = if os == Os::Windows { ".exe" } else { "" };
            let (server, admin) = match engine {
                Engine::Mysql => ("mysqld", "mysqladmin"),
                Engine::Mariadb => ("mariadbd", "mariadb-admin"),
            };

            // Who and where.
            assert_eq!(spec.id, format!("{runtime}@{line}"));
            assert_eq!(d.engine, engine);
            assert_eq!(d.line, line);
            assert_eq!(
                spec.program,
                installed.dir.join("bin").join(format!("{server}{suffix}"))
            );
            assert_eq!(
                d.data_dir,
                bench.home.data_dir().join(format!("{runtime}-{line}"))
            );
            // The port: from the allocator, saved, and the one the supervisor checks.
            assert_eq!(ports.assignment(&spec.id).unwrap().unwrap().port, d.port);
            assert_eq!(spec.ports, [d.port]);
            // The config, named for the platform, and the server told to read it first.
            let config_name = if os == Os::Windows {
                "my.ini"
            } else {
                "my.cnf"
            };
            assert_eq!(
                d.config_file,
                bench
                    .home
                    .config_dir()
                    .join(runtime)
                    .join(line)
                    .join(config_name)
            );
            assert_eq!(
                spec.args,
                [format!("--defaults-file={}", d.config_file.display())]
            );
            // Ready check and clean shutdown: the admin tool over TCP, ignoring any other my.ini on the system.
            let (ready_program, ready_args, stop_args) = helper_args(&d);
            assert_eq!(
                ready_program,
                installed.dir.join("bin").join(format!("{admin}{suffix}"))
            );
            let port = d.port.to_string();
            let expected = |verb: &str| {
                [
                    "--no-defaults",
                    "--protocol=TCP",
                    "-h",
                    "127.0.0.1",
                    "-P",
                    port.as_str(),
                    "-u",
                    "root",
                    verb,
                ]
                .map(str::to_owned)
                .to_vec()
            };
            assert_eq!(ready_args, expected("ping"));
            assert_eq!(stop_args, expected("shutdown"));
            assert_eq!(spec.stop.grace, Duration::from_secs(30));
            assert!(!spec.restart.recycle_on_exit);
            // The error log is a tab of its own.
            assert_eq!(spec.log_files.len(), 1);
            assert_eq!(spec.log_files[0].source, format!("{label} error log"));
            assert_eq!(
                spec.log_files[0].path,
                bench.home.logs_dir().join(format!("{runtime}-{line}.err"))
            );
            // Never set up: the first start gets the long timeout.
            assert!(d.needs_initialisation());
            assert_eq!(spec.start_timeout, Duration::from_secs(120));

            // It is a spec the supervisor accepts, and the files were written.
            spec.validate(&bench.home)
                .unwrap_or_else(|e| panic!("{runtime} {os:?}: {e}"));
            assert!(d.config_file.is_file() && d.user_config_file.is_file());
        }
    }
}

#[test]
fn the_generated_config_holds_this_databases_own_paths_and_port() {
    let bench = private("db-config");
    let ports = allocator(&bench.home);
    let installed = installed(&bench.home, "mariadb", "11.8.9", Os::Windows);

    let d = define(&bench.home, &ports, &installed, Os::Windows);
    let text = read_text(&d.config_file);

    let slash = |p: &Path| p.display().to_string().replace('\\', "/");
    assert!(
        text.contains(&format!("basedir = \"{}\"", slash(&installed.dir))),
        "{text}"
    );
    assert!(text.contains(&format!("datadir = \"{}\"", slash(&d.data_dir))));
    assert!(text.contains(&format!("port = {}\n", d.port)));
    assert!(text.contains(&format!("log-error = \"{}\"", slash(&d.error_log))));
    assert!(text.contains("bind-address = 127.0.0.1"), "FR-SV-08");
    assert!(text.contains(&format!("!include {}", slash(&d.user_config_file))));
    assert!(!text.contains("socket"), "none on Windows");
    // The path of this test's data folder has a space and a non-ASCII letter, and they are in the file as they are.
    assert!(text.contains("lüna home"), "{text}");
}

#[cfg(unix)]
#[test]
fn a_unix_config_has_a_socket_in_the_run_folder() {
    let bench = private("db-socket");
    let ports = allocator(&bench.home);
    let installed = installed(&bench.home, "mysql", "8.4.13", Os::Linux);

    let d = define(&bench.home, &ports, &installed, Os::Linux);
    let text = read_text(&d.config_file);

    assert!(text.contains(&format!(
        "socket = \"{}\"",
        bench.home.run_dir().join("mysql-8.4.sock").display()
    )));
}

#[test]
fn the_line_and_not_the_patch_version_names_the_service_its_data_its_config_and_its_port() {
    let bench = private("db-line");
    let ranges = own_ranges();
    let ports = PortAllocator::new(bench.home.clone()).with_ranges(ranges.clone());
    let older = define(
        &bench.home,
        &ports,
        &installed(&bench.home, "mysql", "8.4.12", Os::Windows),
        Os::Windows,
    );
    let newer = define(
        &bench.home,
        &ports,
        &installed(&bench.home, "mysql", "8.4.13", Os::Windows),
        Os::Windows,
    );
    let other_line = define(
        &bench.home,
        &ports,
        &installed(&bench.home, "mysql", "8.0.46", Os::Windows),
        Os::Windows,
    );

    assert_eq!(older.spec.id, newer.spec.id);
    assert_eq!(
        (
            older.data_dir.clone(),
            older.config_file.clone(),
            older.port
        ),
        (newer.data_dir, newer.config_file, newer.port)
    );
    // Another line has its own, so the two can run at once (FR-SV-02, FR-SV-09).
    assert_eq!(other_line.spec.id, "mysql@8.0");
    assert_ne!(other_line.data_dir, older.data_dir);
    assert_ne!(other_line.port, older.port);
    // And the same port the next time lunaRun runs.
    let again = define(
        &bench.home,
        &PortAllocator::new(bench.home.clone()).with_ranges(ranges),
        &installed(&bench.home, "mysql", "8.4.13", Os::Windows),
        Os::Windows,
    );
    assert_eq!(again.port, older.port);
}

#[test]
fn the_generated_config_is_written_again_every_time_and_the_users_file_never() {
    let bench = private("db-rewrite");
    let ports = allocator(&bench.home);
    let installed = installed(&bench.home, "mysql", "8.4.13", Os::Windows);
    let first = define(&bench.home, &ports, &installed, Os::Windows);
    assert_eq!(
        fs::read_to_string(&first.user_config_file).unwrap(),
        "# Your own MySQL settings. lunaRun never overwrites this file.\n[mysqld]\n"
    );
    fs::write(&first.config_file, "somebody edited the generated file").unwrap();
    fs::write(&first.user_config_file, "[mysqld]\nmax_connections = 500\n").unwrap();
    let moved = ports
        .reassign("mysql@8.4", luna_core::services::PortPurpose::Database)
        .unwrap();

    let second = define(&bench.home, &ports, &installed, Os::Windows);

    assert_eq!(second.port, moved.port);
    let text = read_text(&second.config_file);
    assert!(
        text.contains(&format!("port = {}\n", moved.port)),
        "the generated file follows the port: {text}"
    );
    assert!(!text.contains("somebody edited"), "and loses hand edits");
    assert_eq!(
        fs::read_to_string(&second.user_config_file).unwrap(),
        "[mysqld]\nmax_connections = 500\n",
        "the user's file is theirs"
    );
}

#[test]
fn only_mysql_8_and_later_gets_the_x_protocol_switched_off() {
    let bench = private("db-mysqlx");
    let ports = allocator(&bench.home);
    let text = |runtime: &str, version: &str| {
        let d = define(
            &bench.home,
            &ports,
            &installed(&bench.home, runtime, version, Os::Windows),
            Os::Windows,
        );
        read_text(&d.config_file)
    };

    assert!(
        text("mysql", "8.4.13").contains("mysqlx = OFF"),
        "else a second MySQL cannot take 33060"
    );
    assert!(text("mysql", "9.7.4").contains("mysqlx = OFF"));
    assert!(
        !text("mysql", "5.7.44").contains("mysqlx ="),
        "5.7 does not know the option and would not start"
    );
    assert!(!text("mariadb", "11.8.9").contains("mysqlx ="));
}

#[test]
fn something_that_is_not_a_database_or_has_no_version_is_refused() {
    let bench = private("db-refused");
    let ports = allocator(&bench.home);
    let mut node = installed(&bench.home, "mysql", "8.4.13", Os::Windows);
    node.runtime = "node".to_owned();
    assert_eq!(
        database_definition(&bench.home, &ports, &node, Os::Windows)
            .unwrap_err()
            .code,
        ErrorCode::Internal
    );

    let mut odd = installed(&bench.home, "mysql", "8.4.13", Os::Windows);
    odd.version = "latest".to_owned();
    assert_eq!(
        database_definition(&bench.home, &ports, &odd, Os::Windows)
            .unwrap_err()
            .code,
        ErrorCode::Internal
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_on_a_port_another_program_holds_is_refused_before_anything_runs() {
    let bench = private("db-port-taken");
    let (supervisor, events) = bench.supervisor();
    let ports = allocator(&bench.home);
    let installed = installed(&bench.home, "mysql", "8.4.13", Os::current());
    let d = define(&bench.home, &ports, &installed, Os::current());
    // XAMPP's mysqld, say: listening on every address of the port the database was given.
    let _xampp = TcpListener::bind(("0.0.0.0", d.port)).unwrap();

    let error = supervisor.start(d.spec.clone()).unwrap_err();

    assert_eq!(error.code, ErrorCode::PortInUse);
    assert_eq!(error.details.unwrap()["port"], d.port);
    assert!(supervisor.status("mysql@8.4").is_none() && events.lock().unwrap().is_empty());
    // The user takes another port; the config follows it.
    let moved = ports
        .reassign("mysql@8.4", luna_core::services::PortPurpose::Database)
        .unwrap();
    let again = define(&bench.home, &ports, &installed, Os::current());
    assert_eq!(
        (again.port, again.spec.ports.clone()),
        (moved.port, vec![moved.port])
    );
}

// ---------------------------------------------------------------------------------------------
// First start
// ---------------------------------------------------------------------------------------------

/// A definition whose setup program is the stand-in in `mode`.
fn with_setup(bench: &Shared, mode: &str, extra: &[(&str, &str)]) -> (DatabaseDefinition, PathBuf) {
    let ports = allocator(&bench.home);
    let installed = installed(&bench.home, "mariadb", "11.8.9", Os::current());
    let mut definition = define(&bench.home, &ports, &installed, Os::current());
    let counter = bench.scratch("t").join("setup-runs");
    let data = definition.data_dir.display().to_string();
    let mut env = vec![
        ("LUNA_STUB_DATADIR", data.as_str()),
        ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
    ];
    env.extend_from_slice(extra);
    definition.init = bench.helper(mode, &env);
    (definition, counter)
}

fn runs(counter: &Path) -> usize {
    fs::read_to_string(counter).map_or(0, |text| text.lines().count())
}

const LIMIT: Duration = Duration::from_secs(30);

#[test]
fn a_data_folder_that_is_missing_or_empty_needs_setting_up_and_one_with_files_does_not() {
    let bench = private("db-needs-init");
    let (d, _) = with_setup(&bench, "init-ok", &[]);
    assert!(d.needs_initialisation(), "missing");
    fs::create_dir_all(&d.data_dir).unwrap();
    assert!(d.needs_initialisation(), "empty");
    fs::write(d.data_dir.join("ibdata1"), "x").unwrap();
    assert!(!d.needs_initialisation());

    // The start timeout of a database that is already set up is the short one.
    let rebuilt = define(
        &bench.home,
        &allocator(&bench.home),
        &installed(&bench.home, "mariadb", "11.8.9", Os::current()),
        Os::current(),
    );
    assert_eq!(rebuilt.spec.start_timeout, Duration::from_secs(30));
}

#[tokio::test(flavor = "multi_thread")]
async fn setting_up_makes_the_data_folder_once_and_keeps_the_output() {
    let bench = private("db-init-ok");
    let (d, counter) = with_setup(&bench, "init-ok", &[]);

    d.initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap();

    assert!(!d.needs_initialisation());
    assert!(d.data_dir.join("mysql").join("user.MYD").exists());
    assert_eq!(runs(&counter), 1);
    assert!(
        fs::read_to_string(&d.init_log)
            .unwrap()
            .contains("initialised"),
        "its output is kept"
    );
    // Before every start is fine: with data in place it does nothing.
    d.initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(runs(&counter), 1, "the setup program was not run again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setup_that_fails_says_why_removes_what_it_made_and_can_be_tried_again() {
    let bench = private("db-init-fail");
    let (mut d, counter) = with_setup(&bench, "init-fail", &[]);

    let error = d
        .initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::DatabaseInitFailed);
    assert!(
        error.message.contains("MariaDB 11.8") && error.message.contains("exit code 3"),
        "{}",
        error.message
    );
    let details = error.details.unwrap();
    assert_eq!(details["service"], "mariadb@11.8");
    assert!(
        details["tail"]
            .to_string()
            .contains("boom: cannot create the system tables"),
        "{details}"
    );
    assert_eq!(Path::new(details["log"].as_str().unwrap()), d.init_log);
    assert!(!d.data_dir.exists(), "the half-made data folder is gone");
    assert_eq!(runs(&counter), 1);

    // The next try starts clean.
    let data = d.data_dir.display().to_string();
    d.init = bench.helper("init-ok", &[("LUNA_STUB_DATADIR", data.as_str())]);
    d.initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap();
    assert!(!d.needs_initialisation());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setup_that_finishes_but_makes_nothing_is_a_failure() {
    let bench = private("db-init-empty");
    let (d, _) = with_setup(&bench, "init-empty", &[]);

    let error = d
        .initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::DatabaseInitFailed);
    assert!(
        error.message.contains("made no data folder"),
        "{}",
        error.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setup_that_never_ends_is_stopped_at_the_limit() {
    let bench = private("db-init-timeout");
    let heartbeat = bench.scratch("t").join("heartbeat");
    // `serve` with nothing to listen on runs until it is killed.
    let (d, _) = with_setup(
        &bench,
        "serve",
        &[
            ("LUNA_STUB_NO_LISTEN", "1"),
            ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
        ],
    );
    let begun = std::time::Instant::now();

    let error = d
        .initialise(Duration::from_millis(700), &CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::DatabaseInitFailed);
    assert!(
        error.message.contains("did not finish within"),
        "{}",
        error.message
    );
    assert!(begun.elapsed() < Duration::from_secs(10));
    assert_process_gone(&heartbeat).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setup_can_be_cancelled_and_leaves_nothing_behind() {
    let bench = private("db-init-cancel");
    let heartbeat = bench.scratch("t").join("heartbeat");
    let (d, _) = with_setup(
        &bench,
        "serve",
        &[
            ("LUNA_STUB_NO_LISTEN", "1"),
            ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
        ],
    );
    // Something it made before it was cancelled.
    fs::create_dir_all(&d.data_dir).unwrap();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        trigger.cancel();
    });

    let error = d.initialise(LIMIT, &cancel).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::Cancelled);
    assert!(!d.data_dir.exists());
    assert_process_gone(&heartbeat).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setup_program_that_is_missing_is_a_failure_not_a_panic_and_existing_data_needs_none() {
    let bench = private("db-init-missing");
    let (mut d, _) = with_setup(&bench, "init-ok", &[]);
    d.init.program = bench.home.runtimes_dir().join("not-there.exe");

    let error = d
        .initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DatabaseInitFailed);
    assert!(
        error.message.contains("could not be started"),
        "{}",
        error.message
    );

    // With data in place the (missing) program is never needed.
    fs::create_dir_all(&d.data_dir).unwrap();
    fs::write(d.data_dir.join("ibdata1"), "x").unwrap();
    d.initialise(LIMIT, &CancellationToken::new())
        .await
        .unwrap();
}
