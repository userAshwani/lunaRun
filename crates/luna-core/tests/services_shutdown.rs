//! Stopping everything on Quit: the order (web, then PHP, then the databases), services of one group stopping at the
//! same time, what the report says, and a Quit that cannot be held up (D-029). The services are stand-ins with the ids
//! the real definitions use.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use luna_core::ErrorCode;
use luna_core::services::{
    ServiceSpec, ServiceState, ShutdownGroup, ShutdownProgress, StopMethod, group_of,
};
use stub_service::*;

const LIMIT: Duration = Duration::from_secs(30);

/// A stand-in with the given id that leaves when asked, after `leave_ms` (it finishes what it is doing first).
fn polite(bench: &Shared, id: &str, leave_ms: u64) -> ServiceSpec {
    let stop_file = stop_file(bench, id);
    let delay = leave_ms.to_string();
    let mut spec = bench.serving(
        id,
        free_port(),
        &[
            ("LUNA_STUB_STOPFILE", stop_file.to_str().unwrap()),
            ("LUNA_STUB_LEAVE_DELAY_MS", &delay),
        ],
    );
    spec.stop.method = StopMethod::Command(
        bench.helper("touch", &[("LUNA_STUB_FILE", stop_file.to_str().unwrap())]),
    );
    spec.stop.grace = Duration::from_secs(20);
    spec
}

fn stop_file(bench: &Shared, id: &str) -> PathBuf {
    bench.scratch("stops").join(format!("{id}.stop"))
}

/// A stand-in that does not leave when asked: its stop command does nothing.
fn stubborn(bench: &Shared, id: &str, grace: Duration) -> (ServiceSpec, PathBuf) {
    let heartbeat = bench.scratch("beats").join(format!("{id}.beat"));
    let stop_file = stop_file(bench, id);
    let mut spec = bench.serving(
        id,
        free_port(),
        &[
            ("LUNA_STUB_STOPFILE", stop_file.to_str().unwrap()),
            ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
        ],
    );
    spec.stop.method = StopMethod::Command(bench.helper("noop", &[]));
    spec.stop.grace = grace;
    (spec, heartbeat)
}

fn index_of(events: &Events, id: &str, state: ServiceState, last: bool) -> usize {
    let events = events.lock().unwrap();
    let mut matches = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.id == id && e.state == state)
        .map(|(i, _)| i);
    let found = if last {
        matches.next_back()
    } else {
        matches.next()
    };
    found.unwrap_or_else(|| panic!("no {state:?} event for {id}"))
}

// ---------------------------------------------------------------------------------------------
// Which group
// ---------------------------------------------------------------------------------------------

#[test]
fn every_kind_of_service_id_is_in_its_group() {
    assert_eq!(group_of("caddy"), ShutdownGroup::Web);
    for id in [
        "php-fcgi@8.3.36-w1",
        "php-fcgi@8.5.0-rc1-w4",
        "php-fpm@8.3.36",
    ] {
        assert_eq!(group_of(id), ShutdownGroup::Php, "{id}");
    }
    for id in ["mysql@8.4", "mariadb@12.3", "mysql@9.7"] {
        assert_eq!(group_of(id), ShutdownGroup::Databases, "{id}");
    }
    // What nobody knows goes with the first group; a look-alike is not PHP or a database.
    for id in [
        "worker@shop-queue",
        "php",
        "phpx@1",
        "mysql",
        "my-mysql@8.4",
    ] {
        assert_eq!(group_of(id), ShutdownGroup::Web, "{id}");
    }
    assert_eq!(
        ShutdownGroup::ORDER,
        [
            ShutdownGroup::Web,
            ShutdownGroup::Php,
            ShutdownGroup::Databases
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// The order
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_quit_stops_the_web_side_then_php_then_the_databases() {
    let bench = private("shutdown-order");
    let (supervisor, events) = bench.supervisor();
    let ids = [
        "mysql@8.4",
        "php-fcgi@8.3.36-w1",
        "caddy",
        "mariadb@12.3",
        "php-fpm@8.4.1",
    ];
    for id in ids {
        supervisor.start(polite(&bench, id, 300)).unwrap();
    }
    for id in ids {
        settled(&supervisor, id).await;
    }

    let mut progress: Vec<ShutdownProgress> = Vec::new();
    let report = supervisor
        .shutdown_all(LIMIT, |p| progress.push(p.clone()))
        .await;

    // Everything left by itself, nothing is running.
    let mut clean = report.clean.clone();
    clean.sort();
    let mut expected = ids.map(str::to_owned).to_vec();
    expected.sort();
    assert_eq!(clean, expected);
    assert!(
        report.killed.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );
    for status in supervisor.list() {
        assert_eq!(status.state, ServiceState::Stopped, "{}", status.id);
        assert!(!status.killed);
    }

    // The order, from the events as they happened: a group starts stopping only when the one before is gone.
    let web_gone = index_of(&events, "caddy", ServiceState::Stopped, true);
    for php in ["php-fcgi@8.3.36-w1", "php-fpm@8.4.1"] {
        assert!(
            web_gone < index_of(&events, php, ServiceState::Stopping, false),
            "{php} was asked to stop before Caddy was gone"
        );
    }
    let php_gone = ["php-fcgi@8.3.36-w1", "php-fpm@8.4.1"]
        .map(|php| index_of(&events, php, ServiceState::Stopped, true))
        .into_iter()
        .max()
        .unwrap();
    for database in ["mysql@8.4", "mariadb@12.3"] {
        assert!(
            php_gone < index_of(&events, database, ServiceState::Stopping, false),
            "{database} was asked to stop before PHP was gone"
        );
    }

    // The progress: each group announces itself, then once per service, counting up to its total.
    let groups: Vec<ShutdownGroup> = {
        let mut seen: Vec<ShutdownGroup> = Vec::new();
        for p in &progress {
            if seen.last() != Some(&p.group) {
                seen.push(p.group);
            }
        }
        seen
    };
    assert_eq!(groups, ShutdownGroup::ORDER);
    for (group, total) in [
        (ShutdownGroup::Web, 1),
        (ShutdownGroup::Php, 2),
        (ShutdownGroup::Databases, 2),
    ] {
        let mine: Vec<&ShutdownProgress> = progress.iter().filter(|p| p.group == group).collect();
        assert_eq!(
            mine.len(),
            total as usize + 1,
            "{group:?}: start and one per service"
        );
        assert_eq!(mine[0].done, 0);
        assert_eq!(mine[0].finished, None);
        assert_eq!(mine[0].total, total);
        assert_eq!(mine[0].remaining.len(), total as usize);
        let last = mine.last().unwrap();
        assert_eq!((last.done, last.total), (total, total));
        assert!(last.remaining.is_empty());
        assert!(last.finished.is_some());
        let counts: Vec<u32> = mine.iter().map(|p| p.done).collect();
        assert!(counts.windows(2).all(|w| w[0] + 1 == w[1]), "{counts:?}");
    }
    assert!(report.elapsed_ms >= 900, "three groups of 300 ms each");
}

#[tokio::test(flavor = "multi_thread")]
async fn services_of_one_group_stop_at_the_same_time() {
    let bench = private("shutdown-parallel");
    let (supervisor, _events) = bench.supervisor();
    let ids = [
        "php-fcgi@8.3.36-w1",
        "php-fcgi@8.3.36-w2",
        "php-fcgi@8.3.36-w3",
    ];
    for id in ids {
        supervisor.start(polite(&bench, id, 1000)).unwrap();
    }
    for id in ids {
        settled(&supervisor, id).await;
    }

    let begun = Instant::now();
    let report = supervisor.shutdown_all(LIMIT, |_| {}).await;

    assert_eq!(report.clean.len(), 3, "{report:?}");
    assert!(
        begun.elapsed() < Duration::from_millis(2500),
        "three services that take a second each stopped one after the other: {:?}",
        begun.elapsed()
    );
    assert!(begun.elapsed() >= Duration::from_millis(1000));
}

// ---------------------------------------------------------------------------------------------
// Nothing to do, and the odd ones
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn with_nothing_running_a_quit_is_quick_and_silent() {
    let bench = private("shutdown-nothing");
    let (supervisor, _events) = bench.supervisor();

    let mut calls = 0;
    let report = supervisor.shutdown_all(LIMIT, |_| calls += 1).await;
    assert_eq!(calls, 0);
    assert!(report.clean.is_empty() && report.killed.is_empty() && report.failed.is_empty());

    // Services that were stopped already are not stopped again, and not reported.
    supervisor.start(polite(&bench, "caddy", 0)).unwrap();
    settled(&supervisor, "caddy").await;
    supervisor.stop("caddy").await.unwrap();
    let report = supervisor.shutdown_all(LIMIT, |_| calls += 1).await;
    assert_eq!(calls, 0);
    assert!(report.clean.is_empty(), "{report:?}");

    // And asking twice is the same as asking once.
    supervisor.start(polite(&bench, "caddy", 0)).unwrap();
    settled(&supervisor, "caddy").await;
    assert_eq!(
        supervisor.shutdown_all(LIMIT, |_| {}).await.clean,
        ["caddy"]
    );
    assert!(
        supervisor
            .shutdown_all(LIMIT, |_| {})
            .await
            .clean
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_ignores_its_stop_is_killed_and_the_report_says_so() {
    let bench = private("shutdown-killed");
    let (supervisor, _events) = bench.supervisor();
    let (database, heartbeat) = stubborn(&bench, "mysql@8.4", Duration::from_millis(800));
    supervisor
        .start(polite(&bench, "php-fpm@8.3.36", 0))
        .unwrap();
    supervisor.start(database.clone()).unwrap();
    settled(&supervisor, "php-fpm@8.3.36").await;
    settled(&supervisor, "mysql@8.4").await;

    let report = supervisor.shutdown_all(LIMIT, |_| {}).await;

    assert_eq!(report.clean, ["php-fpm@8.3.36"]);
    assert_eq!(report.killed, ["mysql@8.4"]);
    assert!(report.failed.is_empty(), "{report:?}");
    let status = supervisor.status("mysql@8.4").unwrap();
    assert_eq!(status.state, ServiceState::Stopped);
    assert!(status.killed, "the status says it was killed");
    assert!(!supervisor.status("php-fpm@8.3.36").unwrap().killed);
    assert_process_gone(&heartbeat).await;

    // A new start forgets how the last run ended.
    supervisor.start(database).unwrap();
    let again = settled(&supervisor, "mysql@8.4").await;
    assert_eq!(again.state, ServiceState::Running);
    assert!(!again.killed);
    supervisor.shutdown_all(LIMIT, |_| {}).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crashed_service_waiting_to_restart_is_stopped_too_and_does_not_come_back() {
    let bench = private("shutdown-crashed");
    let (supervisor, _events) = bench.supervisor();
    let counter = bench.scratch("t").join("launches");
    let mut spec = bench.serving(
        "php-fpm@8.3.36",
        free_port(),
        &[
            ("LUNA_STUB_EXIT_AFTER_MS", "200"),
            ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
        ],
    );
    // It crashes, then waits five seconds before it would start again.
    spec.restart.backoff = vec![Duration::from_secs(5)];
    supervisor.start(spec).unwrap();
    supervisor
        .wait_until("php-fpm@8.3.36", LIMIT, |s| {
            s.state == ServiceState::Crashed
        })
        .await
        .unwrap();

    let report = supervisor.shutdown_all(LIMIT, |_| {}).await;

    assert_eq!(report.clean, ["php-fpm@8.3.36"], "{report:?}");
    assert_eq!(
        supervisor.status("php-fpm@8.3.36").unwrap().state,
        ServiceState::Stopped
    );
    // Well past the pause it was waiting out: it was not started again in the middle of the Quit.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        supervisor.status("php-fpm@8.3.36").unwrap().state,
        ServiceState::Stopped
    );
    assert_eq!(launches(&counter), 1, "it ran once");
}

// ---------------------------------------------------------------------------------------------
// A Quit that cannot be held up
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_will_not_stop_is_reported_and_the_next_group_goes_ahead() {
    let bench = private("shutdown-limit");
    let (supervisor, _events) = bench.supervisor();
    // Its stop does nothing and its grace period is long: left to itself it would take half a minute.
    let (stuck, _) = stubborn(&bench, "php-fpm@8.3.36", Duration::from_secs(30));
    supervisor.start(stuck).unwrap();
    supervisor.start(polite(&bench, "mysql@8.4", 0)).unwrap();
    settled(&supervisor, "php-fpm@8.3.36").await;
    settled(&supervisor, "mysql@8.4").await;

    let begun = Instant::now();
    let report = supervisor
        .shutdown_all(Duration::from_secs(1), |_| {})
        .await;

    assert!(
        begun.elapsed() < Duration::from_secs(10),
        "{:?}",
        begun.elapsed()
    );
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(report.failed[0].id, "php-fpm@8.3.36");
    assert_eq!(report.failed[0].error.code, ErrorCode::ServiceStopFailed);
    assert!(
        report.failed[0]
            .error
            .message
            .contains("still running after 1 s"),
        "{}",
        report.failed[0].error.message
    );
    // The database was not held up by it.
    assert_eq!(report.clean, ["mysql@8.4"]);
    assert_eq!(
        supervisor.status("mysql@8.4").unwrap().state,
        ServiceState::Stopped
    );

    // Let the stuck one go (the stand-in leaves when its stop file appears), so nothing is left running.
    fs::write(stop_file(&bench, "php-fpm@8.3.36"), "now").unwrap();
    supervisor
        .wait_until("php-fpm@8.3.36", LIMIT, |s| {
            s.state == ServiceState::Stopped
        })
        .await
        .unwrap();
}
