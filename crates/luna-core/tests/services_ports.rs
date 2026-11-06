//! Ports for services: stable assignments from the ranges of SERVICES section 3, the check before a start, and who
//! holds a port that is taken (FR-SV-03). Real sockets and real processes; every data folder has a space and a
//! non-ASCII letter in its path (NFR-07). The stand-in service is in `stub_service/`.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::collections::HashSet;
use std::fs;
use std::net::TcpListener;
use std::time::Duration;

use luna_core::services::{
    PortAllocator, PortPurpose, PortRanges, check_ports, port_owners, same_program,
};
use luna_core::{ErrorCode, LunaError};
use serde_json::Value;
use stub_service::*;

/// Small ranges of free ports, so the tests do not depend on what runs on this machine.
fn ranges() -> PortRanges {
    PortRanges {
        site: free_block(6),
        php_fastcgi: free_block(8),
        database: free_block(4),
        caddy_admin: free_block(1),
    }
}

fn allocator(test: &str) -> (Shared, PortAllocator) {
    let bench = private(test);
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(ranges());
    (bench, allocator)
}

/// Listen on a port of the shared registry, on `addr`, and keep listening until the value is dropped. (A port the OS
/// chooses would not be known to `free_port`, which could then hand the same one to another test.)
fn hold(addr: &str) -> (TcpListener, u16) {
    let port = free_port();
    (TcpListener::bind((addr, port)).unwrap(), port)
}

fn in_use(error: &LunaError) -> &serde_json::Map<String, Value> {
    assert_eq!(error.code, ErrorCode::PortInUse, "{error}");
    error.details.as_ref().unwrap().as_object().unwrap()
}

// ---------------------------------------------------------------------------------------------
// Is a port free, and who has it
// ---------------------------------------------------------------------------------------------

#[test]
fn a_free_port_passes_the_check_and_has_no_owner() {
    let port = free_port();

    check_ports(&[port]).unwrap();
    check_ports(&[]).unwrap();
    assert!(port_owners(port).is_empty());
}

#[test]
fn a_port_this_process_listens_on_is_in_use_and_the_error_names_this_process() {
    let (_listener, port) = hold("127.0.0.1");

    let error = check_ports(&[free_port(), port]).unwrap_err();

    let details = in_use(&error);
    assert_eq!(details["port"], port);
    assert_eq!(details["pid"], std::process::id());
    let path = details["processPath"].as_str().unwrap();
    assert!(
        same_program(path.as_ref(), &std::env::current_exe().unwrap()),
        "{path}"
    );
    // The name is the file name of the path the OS reported, not necessarily of `current_exe`: the stand-in service
    // of the other tests is a hard link of this very program, and macOS may name any of its links (D-024). That it
    // is this program was checked above by file identity.
    let name = std::path::Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(details["processName"], name);
    assert!(
        error
            .message
            .contains(&format!("port {port} is already in use by {name}")),
        "{}",
        error.message
    );
}

/// The case a plain bind test misses on Windows and macOS: another program listens on the wildcard address (as XAMPP's
/// `mysqld` does), and binding the loopback address of the same port still succeeds.
#[test]
fn a_port_held_on_the_wildcard_address_is_found_too() {
    let (_listener, port) = hold("0.0.0.0");

    let error = check_ports(&[port]).unwrap_err();

    assert_eq!(in_use(&error)["pid"], std::process::id());
    assert_eq!(port_owners(port).len(), 1);
}

#[test]
fn a_listener_on_the_ipv6_loopback_does_not_clash_with_127_0_0_1() {
    // Not every machine has IPv6 loopback; where it does not there is nothing to check.
    let Ok(listener) = TcpListener::bind(("::1", 0)) else {
        return;
    };
    let port = listener.local_addr().unwrap().port();

    check_ports(&[port]).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_held_by_another_program_names_that_program() {
    let bench = private("ports-other-program");
    let port = free_port();
    let holder = detached(&bench, port, &[]);
    for _ in 0..400 {
        if port_open(port) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let error = check_ports(&[port]).unwrap_err();

    let details = in_use(&error);
    assert_eq!(details["pid"], holder.pid);
    let reported = details["processPath"].as_str().unwrap();
    assert!(same_program(reported.as_ref(), &bench.stub));
    // The name of whichever link of the stand-in the OS reported (macOS may name any of them, D-024).
    assert_eq!(
        details["processName"],
        std::path::Path::new(reported)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    let owners = port_owners(port);
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].pid, holder.pid);
}

// ---------------------------------------------------------------------------------------------
// Assigning
// ---------------------------------------------------------------------------------------------

#[test]
fn each_purpose_gets_a_port_from_its_own_range() {
    let bench = private("ports-ranges");
    let ranges = ranges();
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(ranges.clone());

    let site = allocator.assign("site@shop", PortPurpose::Site).unwrap();
    let pool = allocator
        .assign("php-fcgi@8.3.14", PortPurpose::PhpFastCgi { pool: 4 })
        .unwrap();
    let db = allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap();
    let admin = allocator.assign("caddy", PortPurpose::CaddyAdmin).unwrap();

    assert_eq!(
        (site.port, site.count),
        (*ranges.site.start(), 1),
        "the first free one"
    );
    assert!(ranges.php_fastcgi.contains(&pool.port) && pool.count == 4);
    assert_eq!(pool.ports(), (pool.port..pool.port + 4).collect::<Vec<_>>());
    assert_eq!(db.port, *ranges.database.start());
    assert_eq!(admin.port, *ranges.caddy_admin.start());
}

#[test]
fn the_shipped_ranges_are_the_ones_in_the_service_spec_document() {
    let ranges = PortRanges::default();

    assert_eq!(ranges.site, 8100..=8999);
    assert_eq!(ranges.php_fastcgi, 9100..=9999);
    assert_eq!(ranges.database, 3306..=3399);
    assert_eq!(ranges.caddy_admin, 2119..=2119);
}

#[test]
fn a_service_keeps_its_ports_across_restarts_and_every_service_has_its_own() {
    let bench = private("ports-stable");
    let ranges = ranges();
    let first = PortAllocator::new(bench.home.clone()).with_ranges(ranges.clone());
    let a = first.assign("mysql@8.4", PortPurpose::Database).unwrap();
    let b = first.assign("mysql@8.0", PortPurpose::Database).unwrap();
    assert_ne!(a.port, b.port);
    assert_eq!(
        first.assign("mysql@8.4", PortPurpose::Database).unwrap(),
        a,
        "asking again changes nothing"
    );

    // lunaRun restarts: a new allocator reads the file.
    let second = PortAllocator::new(bench.home.clone()).with_ranges(ranges);
    assert_eq!(
        second.assign("mysql@8.0", PortPurpose::Database).unwrap(),
        b
    );
    assert_eq!(second.assignment("mysql@8.4").unwrap(), Some(a));
    assert_eq!(second.all().unwrap().len(), 2);
    assert_eq!(second.assignment("nothing").unwrap(), None);
}

#[test]
fn a_pool_gets_consecutive_ports_and_pools_never_overlap() {
    let (_bench, allocator) = allocator("ports-pools");

    let one = allocator
        .assign("php-fcgi@8.3.14", PortPurpose::PhpFastCgi { pool: 4 })
        .unwrap();
    let two = allocator
        .assign("php-fcgi@8.2.20", PortPurpose::PhpFastCgi { pool: 3 })
        .unwrap();

    let (a, b): (HashSet<u16>, HashSet<u16>) = (
        one.ports().into_iter().collect(),
        two.ports().into_iter().collect(),
    );
    assert_eq!((a.len(), b.len()), (4, 3));
    assert!(a.is_disjoint(&b));
    // A pool that changes size is given a new run, not squeezed into the old one.
    let bigger = allocator
        .assign("php-fcgi@8.3.14", PortPurpose::PhpFastCgi { pool: 2 })
        .unwrap();
    assert_eq!(bigger.count, 2);
    assert_eq!(
        allocator.assignment("php-fcgi@8.3.14").unwrap(),
        Some(bigger)
    );
    assert_eq!(
        allocator
            .assign("nothing", PortPurpose::PhpFastCgi { pool: 0 })
            .unwrap_err()
            .code,
        ErrorCode::Internal
    );
}

#[test]
fn ports_that_are_taken_are_skipped() {
    let bench = private("ports-skip-taken");
    let site = free_block(6);
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        site: site.clone(),
        ..ranges()
    });
    // Another program holds the first port; the second one is held on the wildcard address.
    let _first = TcpListener::bind(("127.0.0.1", *site.start())).unwrap();
    let _second = TcpListener::bind(("0.0.0.0", site.start() + 1)).unwrap();

    let got = allocator.assign("site@shop", PortPurpose::Site).unwrap();

    assert_eq!(got.port, site.start() + 2);
}

#[test]
fn a_pool_skips_a_window_that_has_a_taken_port_in_the_middle() {
    let bench = private("ports-skip-window");
    let pool = free_block(8);
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        php_fastcgi: pool.clone(),
        ..ranges()
    });
    let _middle = TcpListener::bind(("127.0.0.1", pool.start() + 1)).unwrap();

    let got = allocator
        .assign("php-fcgi@8.3.14", PortPurpose::PhpFastCgi { pool: 3 })
        .unwrap();

    // The windows starting at +0 and +1 contain the taken port.
    assert_eq!(got.port, pool.start() + 2);
}

#[test]
fn ports_given_to_one_service_are_not_given_to_another_even_when_nothing_listens_on_them() {
    let (_bench, allocator) = allocator("ports-not-twice");
    let a = allocator.assign("site@one", PortPurpose::Site).unwrap();
    let b = allocator.assign("site@two", PortPurpose::Site).unwrap();
    let c = allocator.assign("site@three", PortPurpose::Site).unwrap();

    assert_eq!(HashSet::from([a.port, b.port, c.port]).len(), 3);
}

#[test]
fn a_full_range_is_a_typed_error_and_released_ports_can_be_used_again() {
    let bench = private("ports-exhausted");
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        database: free_block(2),
        ..ranges()
    });
    allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap();
    let second = allocator
        .assign("mysql@8.0", PortPurpose::Database)
        .unwrap();

    let error = allocator
        .assign("mariadb@11.4", PortPurpose::Database)
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::NoFreePort);
    let details = error.details.unwrap();
    assert_eq!(details["service"], "mariadb@11.4");
    assert_eq!(details["count"], 1);

    allocator.release("mysql@8.0").unwrap();
    allocator.release("never-existed").unwrap();
    assert_eq!(
        allocator
            .assign("mariadb@11.4", PortPurpose::Database)
            .unwrap(),
        second
    );
}

#[test]
fn a_range_that_is_full_because_of_other_programs_is_full_too() {
    let bench = private("ports-busy-range");
    let database = free_block(2);
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        database: database.clone(),
        ..ranges()
    });
    let _a = TcpListener::bind(("127.0.0.1", *database.start())).unwrap();
    let _b = TcpListener::bind(("127.0.0.1", *database.end())).unwrap();

    let error = allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::NoFreePort);
    assert_eq!(
        allocator.assignment("mysql@8.4").unwrap(),
        None,
        "nothing was saved"
    );
}

#[test]
fn caddys_admin_port_is_one_port_and_when_it_is_taken_the_error_says_who_has_it() {
    let bench = private("ports-caddy-admin");
    let (_listener, taken) = hold("127.0.0.1");
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        caddy_admin: taken..=taken,
        ..ranges()
    });

    let error = allocator
        .assign("caddy", PortPurpose::CaddyAdmin)
        .unwrap_err();

    // There is no other port to offer, so this is "in use", not "none free", and it names the program.
    let details = in_use(&error);
    assert_eq!(details["port"], taken);
    assert_eq!(details["pid"], std::process::id());
}

#[test]
fn another_port_can_be_asked_for_and_the_old_one_is_released() {
    let bench = private("ports-reassign");
    let database = free_block(4);
    let allocator = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        database: database.clone(),
        ..ranges()
    });
    let first = allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap();
    assert_eq!(first.port, *database.start());
    // 3306 is taken by XAMPP, say: the check says so, and the user takes another one.
    let _xampp = TcpListener::bind(("0.0.0.0", first.port)).unwrap();
    assert_eq!(
        check_ports(&first.ports()).unwrap_err().code,
        ErrorCode::PortInUse
    );

    let other = allocator
        .reassign("mysql@8.4", PortPurpose::Database)
        .unwrap();

    assert_ne!(other.port, first.port);
    assert!(database.contains(&other.port));
    assert_eq!(allocator.assignment("mysql@8.4").unwrap(), Some(other));
    check_ports(&other.ports()).unwrap();
    // Asking again moves on, never back.
    assert_ne!(
        allocator
            .reassign("mysql@8.4", PortPurpose::Database)
            .unwrap()
            .port,
        other.port
    );
}

#[test]
fn two_threads_never_get_the_same_port() {
    let (_bench, allocator) = allocator("ports-threads");

    let handles: Vec<_> = (0..6)
        .map(|n| {
            let allocator = allocator.clone();
            std::thread::spawn(move || {
                allocator
                    .assign(&format!("site@{n}"), PortPurpose::Site)
                    .unwrap()
                    .port
            })
        })
        .collect();
    let ports: HashSet<u16> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert_eq!(ports.len(), 6);
}

// ---------------------------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------------------------

#[test]
fn the_assignments_are_saved_in_config_services_json_with_a_schema() {
    let (bench, allocator) = allocator("ports-file");
    let a = allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap();

    let text = fs::read_to_string(bench.home.config_dir().join("services.json")).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(value["schema"], 1);
    assert_eq!(value["ports"]["mysql@8.4"]["port"], a.port);
    assert_eq!(value["ports"]["mysql@8.4"]["count"], 1);
}

#[test]
fn a_damaged_file_is_set_aside_and_the_allocator_starts_over() {
    let (bench, allocator) = allocator("ports-damaged");
    let file = bench.home.config_dir().join("services.json");
    fs::create_dir_all(bench.home.config_dir()).unwrap();
    fs::write(&file, "this is not json").unwrap();

    let got = allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap();

    assert_eq!(allocator.assignment("mysql@8.4").unwrap(), Some(got));
    let backups = fs::read_dir(bench.home.config_dir())
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("services.json.")
        })
        .count();
    assert_eq!(backups, 1, "the damaged one was kept aside, not lost");
}

#[test]
fn a_file_from_a_newer_lunarun_is_left_alone() {
    let (bench, allocator) = allocator("ports-newer");
    let file = bench.home.config_dir().join("services.json");
    fs::create_dir_all(bench.home.config_dir()).unwrap();
    let newer = r#"{"schema":99,"ports":{},"something":"new"}"#;
    fs::write(&file, newer).unwrap();

    let error = allocator
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::StateSchemaTooNew);
    assert_eq!(fs::read_to_string(&file).unwrap(), newer);
}

#[test]
fn a_service_id_that_is_not_safe_is_refused() {
    let (_bench, allocator) = allocator("ports-ids");
    for bad in ["", "a b", "../x", "a/b"] {
        assert_eq!(
            allocator.assign(bad, PortPurpose::Site).unwrap_err().code,
            ErrorCode::Internal,
            "{bad:?}"
        );
        assert_eq!(
            allocator.reassign(bad, PortPurpose::Site).unwrap_err().code,
            ErrorCode::Internal
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The supervisor checks before it starts
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_service_is_not_started_on_a_port_that_is_taken_and_the_error_says_who_has_it() {
    let bench = private("ports-start-refused");
    let (supervisor, events) = bench.supervisor();
    let (holder, port) = hold("127.0.0.1");

    let error = supervisor
        .start(bench.serving("db", port, &[]))
        .unwrap_err();

    // Immediately, from `start` itself, with the process: not a vague failure after the start timeout.
    let details = in_use(&error);
    assert_eq!(details["port"], port);
    assert_eq!(details["pid"], std::process::id());
    assert!(supervisor.status("db").is_none(), "nothing was created");
    assert!(events.lock().unwrap().is_empty());
    assert!(!pid_file(&bench, "db").exists());

    // The holder lets go, and the same start works.
    drop(holder);
    supervisor.start(bench.serving("db", port, &[])).unwrap();
    settled(&supervisor, "db").await;
    supervisor.stop("db").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_port_of_a_service_is_checked_and_a_service_with_none_is_not() {
    let bench = private("ports-start-many");
    let (supervisor, _events) = bench.supervisor();
    let (_holder, taken) = hold("0.0.0.0");
    let mut pool = bench.serving("php-fcgi@8.3.14", free_port(), &[]);
    pool.ports = vec![free_port(), taken, free_port()];

    let error = supervisor.start(pool).unwrap_err();
    assert_eq!(in_use(&error)["port"], taken, "the second of three");

    // A service that listens on nothing has no port to check.
    let mut worker = bench.serving("worker", taken, &[("LUNA_STUB_NO_LISTEN", "1")]);
    worker.ready = luna_core::services::ReadyCheck::Immediately;
    worker.ports.clear();
    supervisor.start(worker).unwrap();
    settled(&supervisor, "worker").await;
    supervisor.stop("worker").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_runs_is_busy_not_port_in_use_and_the_port_of_a_new_spec_defaults_to_the_ready_port()
 {
    let bench = private("ports-start-busy");
    let (supervisor, _events) = bench.supervisor();
    let port = free_port();
    assert_eq!(
        bench.serving("db", port, &[]).ports,
        [port],
        "the ready-check port is checked by default"
    );
    supervisor.start(bench.serving("db", port, &[])).unwrap();
    settled(&supervisor, "db").await;

    // Its own port is taken, by itself: that is "already running", not a clash.
    let error = supervisor
        .start(bench.serving("db", port, &[]))
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Busy);
    supervisor.stop("db").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_held_by_another_program_stops_a_start_and_names_it() {
    let bench = private("ports-start-other-program");
    let (supervisor, _events) = bench.supervisor();
    let port = free_port();
    let holder = detached(&bench, port, &[]);
    for _ in 0..400 {
        if port_open(port) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let error = supervisor
        .start(bench.serving("db", port, &[]))
        .unwrap_err();

    let details = in_use(&error);
    assert_eq!(details["pid"], holder.pid);
    assert!(same_program(
        details["processPath"].as_str().unwrap().as_ref(),
        &bench.stub
    ));
}

#[test]
fn port_zero_is_not_a_port_a_service_can_listen_on() {
    let bench = private("ports-zero");
    let mut spec = bench.serving("db", free_port(), &[]);
    spec.ports = vec![0];

    assert_eq!(
        spec.validate(&bench.home).unwrap_err().code,
        ErrorCode::Internal
    );
}

// ---------------------------------------------------------------------------------------------
// Probes and process creation at the same time
// ---------------------------------------------------------------------------------------------

/// `check_ports` opens a probe socket for a moment. On Linux and macOS a process created by another thread in that
/// moment inherits it until it execs, which keeps the port **listening** and made later checks (and starts) refuse a free
/// port with `PORT_IN_USE` and no owner (seen on Linux CI). The supervisor takes one lock around both, so no check may
/// ever fail here. On Windows nothing is inherited and this passes trivially.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creating_processes_while_ports_are_checked_never_makes_a_probe_look_like_a_listener() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let bench = private("ports-probe-race");
    let (supervisor, _events) = bench.supervisor();
    let stop = Arc::new(AtomicBool::new(false));
    // Threads that do nothing but check a free port of their own, as fast as they can.
    let checkers: Vec<_> = (0..3)
        .map(|_| {
            let (port, stop) = (free_port(), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut refused = Vec::new();
                let mut checks = 0_u32;
                while !stop.load(Ordering::Relaxed) {
                    checks += 1;
                    if let Err(error) = check_ports(&[port]) {
                        refused.push(error.message);
                    }
                }
                (checks, refused)
            })
        })
        .collect();

    // Meanwhile processes are created over and over, several at once.
    for round in 0..10 {
        let ids: Vec<String> = (0..4).map(|n| format!("race-{round}-{n}")).collect();
        for id in &ids {
            supervisor
                .start(bench.serving(id, free_port(), &[]))
                .unwrap();
        }
        for id in &ids {
            settled(&supervisor, id).await;
        }
        for id in &ids {
            supervisor.stop(id).await.unwrap();
        }
    }

    stop.store(true, Ordering::Relaxed);
    for checker in checkers {
        let (checks, refused) = checker.join().unwrap();
        assert!(checks > 50, "the checker barely ran: {checks} checks");
        assert!(
            refused.is_empty(),
            "a free port was refused {} times in {checks} checks: {:?}",
            refused.len(),
            refused.first()
        );
    }
}
