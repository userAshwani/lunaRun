//! The service catalog: what the Services page lists, built from what is installed, without writing anything or taking a
//! port (D-031), and the answer "which running service is behind this version" that stops an uninstall (D-031). The
//! runtimes are folders with receipts; the running services are the stand-in of the other service tests. Every data
//! folder has a space and a non-ASCII letter in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::runtimes::{InstallReceipt, Uninstaller, list_installed};
use luna_core::services::{
    PortAllocator, PortPurpose, PortRanges, ServiceKind, ServiceRow, ServiceState, Supervisor,
    list_services, running_from, usage_checker,
};
use stub_service::*;

const PLATFORMS: [Os; 2] = [Os::Windows, Os::Linux];

fn own_ranges() -> PortRanges {
    PortRanges {
        php_fastcgi: free_block(16),
        caddy_admin: free_block(1),
        database: free_block(4),
        ..PortRanges::default()
    }
}

fn allocator(home: &LunaHome) -> PortAllocator {
    PortAllocator::new(home.clone()).with_ranges(own_ranges())
}

/// An installed runtime: a folder with a receipt, as the installer leaves it.
fn install(home: &LunaHome, runtime: &str, version: &str) {
    let dir = home.runtime_dir(runtime, version).unwrap();
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::write(dir.join("bin").join("program"), "binary").unwrap();
    InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
        .write(&dir)
        .unwrap();
}

fn list(bench: &Shared, ports: &PortAllocator, supervisor: &Supervisor, os: Os) -> Vec<ServiceRow> {
    let installed = list_installed(&bench.home).unwrap();
    list_services(ports, supervisor, &bench.home, &installed, os).unwrap()
}

fn ids(rows: &[ServiceRow]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

fn row<'a>(rows: &'a [ServiceRow], id: &str) -> &'a ServiceRow {
    rows.iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no row {id}: {:?}", ids(rows)))
}

// ---------------------------------------------------------------------------------------------
// What is listed
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn nothing_installed_is_an_empty_list() {
    let bench = private("catalog-empty");
    let (supervisor, _events) = bench.supervisor();

    assert!(list(&bench, &allocator(&bench.home), &supervisor, Os::current()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_that_runs_nothing_is_not_a_service() {
    let bench = private("catalog-node");
    install(&bench.home, "node", "24.22.2");
    install(&bench.home, "composer", "2.8.1");
    let (supervisor, _events) = bench.supervisor();

    assert!(list(&bench, &allocator(&bench.home), &supervisor, Os::current()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn rows_come_in_the_order_of_the_page_with_the_newest_first() {
    let bench = private("catalog-order");
    for (runtime, version) in [
        ("mysql", "8.4.13"),
        ("mysql", "9.7.4"),
        ("mariadb", "12.3.3"),
        ("mariadb", "11.8.9"),
        ("php", "8.3.36"),
        ("php", "8.4.12"),
        ("php", "8.2.4"),
        ("caddy", "2.11.7"),
    ] {
        install(&bench.home, runtime, version);
    }
    let (supervisor, _events) = bench.supervisor();

    let rows = list(&bench, &allocator(&bench.home), &supervisor, Os::Linux);

    assert_eq!(
        ids(&rows),
        [
            "caddy",
            "php-fpm@8.4.12",
            "php-fpm@8.3.36",
            "php-fpm@8.2.4",
            "mariadb@12.3",
            "mariadb@11.8",
            "mysql@9.7",
            "mysql@8.4"
        ]
    );
    let kinds: Vec<ServiceKind> = rows.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [
            ServiceKind::Web,
            ServiceKind::Php,
            ServiceKind::Php,
            ServiceKind::Php,
            ServiceKind::Database,
            ServiceKind::Database,
            ServiceKind::Database,
            ServiceKind::Database
        ]
    );
    let mysql = row(&rows, "mysql@8.4");
    assert_eq!(
        (mysql.runtime.as_str(), mysql.version.as_str()),
        ("mysql", "8.4.13")
    );
    assert_eq!(mysql.line.as_deref(), Some("8.4"));
    assert_eq!(row(&rows, "caddy").line, None);
    assert_eq!(row(&rows, "php-fpm@8.3.36").line, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_line_runs_from_the_newest_patch_installed() {
    let bench = private("catalog-patches");
    for version in ["8.4.6", "8.4.13", "8.4.9"] {
        install(&bench.home, "mysql", version);
    }
    let (supervisor, _events) = bench.supervisor();

    let rows = list(&bench, &allocator(&bench.home), &supervisor, Os::current());

    assert_eq!(ids(&rows), ["mysql@8.4"], "one row for the line");
    assert_eq!(rows[0].version, "8.4.13");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_windows_pool_is_one_row_of_several_services_and_other_systems_have_one() {
    let bench = private("catalog-pool");
    install(&bench.home, "php", "8.3.36");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);

    let windows = list(&bench, &ports, &supervisor, Os::Windows);
    assert_eq!(ids(&windows), ["php-fcgi@8.3.36"]);
    let member_ids: Vec<&str> = windows[0].members.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        member_ids,
        [
            "php-fcgi@8.3.36-w1",
            "php-fcgi@8.3.36-w2",
            "php-fcgi@8.3.36-w3",
            "php-fcgi@8.3.36-w4"
        ],
        "four workers until the pool is given another size"
    );
    assert_eq!((windows[0].running, windows[0].total), (0, 4));

    let linux = list(&bench, &ports, &supervisor, Os::Linux);
    assert_eq!(ids(&linux), ["php-fpm@8.3.36"]);
    assert_eq!(linux[0].members.len(), 1);
    assert_eq!((linux[0].running, linux[0].total), (0, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pool_shows_the_size_and_the_ports_it_was_given() {
    let bench = private("catalog-pool-size");
    install(&bench.home, "php", "8.3.36");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    let given = ports
        .assign("php-fcgi@8.3.36", PortPurpose::PhpFastCgi { pool: 2 })
        .unwrap();

    let rows = list(&bench, &ports, &supervisor, Os::Windows);

    assert_eq!(rows[0].total, 2);
    assert_eq!(rows[0].ports, given.ports());
}

#[tokio::test(flavor = "multi_thread")]
async fn ports_show_only_once_they_were_given_and_a_database_says_it_needs_setting_up() {
    let bench = private("catalog-ports");
    install(&bench.home, "mysql", "8.4.13");
    install(&bench.home, "caddy", "2.11.7");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);

    let before = list(&bench, &ports, &supervisor, Os::current());
    assert!(row(&before, "mysql@8.4").ports.is_empty());
    assert!(row(&before, "caddy").ports.is_empty());
    assert!(row(&before, "mysql@8.4").needs_setup, "no data folder yet");
    assert!(!row(&before, "caddy").needs_setup);

    let port = ports
        .assign("mysql@8.4", PortPurpose::Database)
        .unwrap()
        .port;
    let data = bench.home.data_dir().join("mysql-8.4");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("ibdata1"), "data").unwrap();

    let after = list(&bench, &ports, &supervisor, Os::current());
    assert_eq!(row(&after, "mysql@8.4").ports, [port]);
    assert!(!row(&after, "mysql@8.4").needs_setup, "it has data now");
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_writes_nothing_and_takes_no_port() {
    let bench = private("catalog-readonly");
    for (runtime, version) in [("mysql", "8.4.13"), ("php", "8.3.36"), ("caddy", "2.11.7")] {
        install(&bench.home, runtime, version);
    }
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    let names = |dir: std::path::PathBuf| -> Vec<String> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let before = (
        names(bench.home.config_dir()),
        names(bench.home.data_dir()),
        names(bench.home.logs_dir()),
        names(bench.home.run_dir()),
    );

    for os in PLATFORMS {
        for _ in 0..3 {
            list(&bench, &ports, &supervisor, os);
        }
    }

    let after = (
        names(bench.home.config_dir()),
        names(bench.home.data_dir()),
        names(bench.home.logs_dir()),
        names(bench.home.run_dir()),
    );
    assert_eq!(before, after, "no config, data, log or PID file was made");
    assert!(
        !bench.home.config_dir().join("services.json").exists(),
        "no port was assigned"
    );
}

// ---------------------------------------------------------------------------------------------
// States come from the supervisor
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_running_service_makes_its_row_running_and_stopping_it_makes_it_stopped() {
    let bench = private("catalog-states");
    install(&bench.home, "mysql", "8.4.13");
    install(&bench.home, "caddy", "2.11.7");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    supervisor
        .start(bench.serving("mysql@8.4", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "mysql@8.4").await;

    let rows = list(&bench, &ports, &supervisor, Os::current());
    let mysql = row(&rows, "mysql@8.4");
    assert_eq!(mysql.state, ServiceState::Running);
    assert_eq!((mysql.running, mysql.total), (1, 1));
    assert!(mysql.members[0].pid.is_some());
    assert_eq!(row(&rows, "caddy").state, ServiceState::Stopped);

    supervisor.stop("mysql@8.4").await.unwrap();
    let rows = list(&bench, &ports, &supervisor, Os::current());
    assert_eq!(row(&rows, "mysql@8.4").state, ServiceState::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pool_is_running_when_all_its_workers_are_and_in_between_it_is_starting() {
    let bench = private("catalog-pool-states");
    install(&bench.home, "php", "8.3.36");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    ports
        .assign("php-fcgi@8.3.36", PortPurpose::PhpFastCgi { pool: 2 })
        .unwrap();
    supervisor
        .start(bench.serving("php-fcgi@8.3.36-w1", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "php-fcgi@8.3.36-w1").await;

    let half = list(&bench, &ports, &supervisor, Os::Windows);
    assert_eq!((half[0].running, half[0].total), (1, 2));
    assert_eq!(half[0].state, ServiceState::Starting);

    supervisor
        .start(bench.serving("php-fcgi@8.3.36-w2", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "php-fcgi@8.3.36-w2").await;
    let all = list(&bench, &ports, &supervisor, Os::Windows);
    assert_eq!((all[0].running, all[0].total), (2, 2));
    assert_eq!(all[0].state, ServiceState::Running);

    supervisor
        .shutdown_all(std::time::Duration::from_secs(30), |_| {})
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_was_killed_says_so_in_its_row() {
    let bench = private("catalog-killed");
    install(&bench.home, "mysql", "8.4.13");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    let mut spec = bench.serving(
        "mysql@8.4",
        free_port(),
        &[(
            "LUNA_STUB_STOPFILE",
            bench.scratch("t").join("never").to_str().unwrap(),
        )],
    );
    spec.stop.method = luna_core::services::StopMethod::Command(bench.helper("noop", &[]));
    spec.stop.grace = std::time::Duration::from_millis(500);
    supervisor.start(spec).unwrap();
    settled(&supervisor, "mysql@8.4").await;
    supervisor.stop("mysql@8.4").await.unwrap();

    let rows = list(&bench, &ports, &supervisor, Os::current());

    assert_eq!(row(&rows, "mysql@8.4").state, ServiceState::Stopped);
    assert!(row(&rows, "mysql@8.4").members[0].killed);
}

// ---------------------------------------------------------------------------------------------
// Which running service is behind a version
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_version_is_in_use_only_by_the_services_that_run_from_it() {
    let bench = private("catalog-usage");
    for (runtime, version) in [
        ("mysql", "8.4.13"),
        ("mysql", "8.4.6"),
        ("php", "8.3.36"),
        ("php", "8.4.12"),
    ] {
        install(&bench.home, runtime, version);
    }
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    let os = Os::Windows;
    let used = |runtime: &str, version: &str| {
        running_from(&ports, &supervisor, &bench.home, os, runtime, version).unwrap()
    };
    assert!(used("mysql", "8.4.13").is_empty(), "nothing runs yet");

    supervisor
        .start(bench.serving("mysql@8.4", free_port(), &[]))
        .unwrap();
    supervisor
        .start(bench.serving("php-fcgi@8.3.36-w2", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "mysql@8.4").await;
    settled(&supervisor, "php-fcgi@8.3.36-w2").await;

    assert_eq!(used("mysql", "8.4.13"), ["mysql@8.4"]);
    assert!(
        used("mysql", "8.4.6").is_empty(),
        "an older patch of the line is not what the service runs from"
    );
    assert_eq!(used("php", "8.3.36"), ["php-fcgi@8.3.36-w2"]);
    assert!(used("php", "8.4.12").is_empty(), "a pool of its own");
    assert!(used("caddy", "2.11.7").is_empty());

    supervisor
        .shutdown_all(std::time::Duration::from_secs(30), |_| {})
        .await;
    assert!(used("mysql", "8.4.13").is_empty(), "stopped again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_is_still_starting_also_blocks_the_uninstall() {
    let bench = private("catalog-usage-starting");
    install(&bench.home, "mysql", "8.4.13");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    // It takes a while to open its port, so for a moment it is `Starting`, not `Running`.
    supervisor
        .start(bench.serving("mysql@8.4", free_port(), &[("LUNA_STUB_DELAY_MS", "1500")]))
        .unwrap();

    let used = running_from(
        &ports,
        &supervisor,
        &bench.home,
        Os::current(),
        "mysql",
        "8.4.13",
    )
    .unwrap();
    let state = supervisor.status("mysql@8.4").unwrap().state;

    assert_eq!(
        state,
        ServiceState::Starting,
        "it has not opened its port yet"
    );
    assert_eq!(used, ["mysql@8.4"], "starting counts: the program is open");
    settled(&supervisor, "mysql@8.4").await;
    supervisor.stop("mysql@8.4").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_uninstaller_is_told_what_runs_so_it_can_refuse() {
    let bench = private("catalog-uninstaller");
    install(&bench.home, "mysql", "8.4.13");
    let (supervisor, _events) = bench.supervisor();
    let ports = allocator(&bench.home);
    let uninstaller = Uninstaller::new(bench.home.clone()).with_usage(usage_checker(
        ports,
        supervisor.clone(),
        bench.home.clone(),
        Os::current(),
        std::sync::Arc::new(|_, _| Vec::new()),
    ));
    assert!(
        uninstaller
            .usage("mysql", "8.4.13")
            .unwrap()
            .running_services
            .is_empty()
    );

    supervisor
        .start(bench.serving("mysql@8.4", free_port(), &[]))
        .unwrap();
    settled(&supervisor, "mysql@8.4").await;

    let usage = uninstaller.usage("mysql", "8.4.13").unwrap();
    assert_eq!(usage.running_services, ["mysql@8.4"]);
    assert!(
        usage.bound_projects.is_empty(),
        "the project store comes later"
    );
    let report = uninstaller.usage_report("mysql", "8.4.13").unwrap();
    assert_eq!(report.running_services, ["mysql@8.4"]);

    supervisor
        .shutdown_all(std::time::Duration::from_secs(30), |_| {})
        .await;
}
