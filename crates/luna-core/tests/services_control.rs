//! Starting, stopping and restarting a row of the Services page (FR-SV-01 to FR-SV-03, FR-SV-10, D-032): what a row id
//! turns into, the first start of a database, a PHP pool that is all or none, Caddy checked before it starts, a start
//! that is cancelled or refused, and the action as a background job. The runtimes are folders with receipts; the
//! programs the plans would run are replaced by the stand-in service through `with_plan_hook`, because the real
//! ones are not there (the real MySQL, MariaDB, PHP and Caddy are run by the ignored `*_online` tests and by the real
//! app window). Every data folder has a space and a non-ASCII letter in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use luna_core::diagnostics::{Arch, Os};
use luna_core::jobs::{JobEvent, JobManager, JobOptions, JobState};
use luna_core::runtimes::{InstallReceipt, list_installed, php_config_paths};
use luna_core::services::{
    PortAllocator, PortPurpose, PortRanges, ServiceAction, ServiceController, ServiceState,
    StartPlan, Supervisor, job_key, list_services,
};
use luna_core::{ErrorCode, LunaHome};
use stub_service::*;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

fn own_ranges() -> PortRanges {
    PortRanges {
        php_fastcgi: free_block(16),
        caddy_admin: free_block(1),
        database: free_block(8),
        ..PortRanges::default()
    }
}

/// An installed runtime: a folder with a receipt, as the installer leaves it (and, for PHP, the `php.ini` it generated).
fn install(home: &LunaHome, runtime: &str, version: &str) {
    let dir = home.runtime_dir(runtime, version).unwrap();
    fs::create_dir_all(dir.join("bin")).unwrap();
    InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
        .write(&dir)
        .unwrap();
    if runtime == "php" {
        let paths = php_config_paths(home, version).unwrap();
        fs::create_dir_all(&paths.dir).unwrap();
        fs::write(&paths.ini, "; generated at install\n").unwrap();
    }
}

fn setup_counter(bench: &Shared) -> PathBuf {
    bench.scratch("control").join("setup-runs")
}

/// The stand-in in place of every real program: it listens on the port the plan chose, its setup program makes the data
/// folder, its `caddy validate` passes.
fn stand_in(plan: &mut StartPlan, bench: &Shared) {
    match plan {
        StartPlan::Database(def) => {
            def.spec = bench.serving(&def.spec.id.clone(), def.port, &[]);
            let data = def.data_dir.display().to_string();
            let counter = setup_counter(bench);
            def.init = bench.helper(
                "init-ok",
                &[
                    ("LUNA_STUB_DATADIR", data.as_str()),
                    ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
                ],
            );
        }
        StartPlan::Php(def) => {
            for spec in &mut def.specs {
                *spec = bench.serving(&spec.id.clone(), spec.ports[0], &[]);
            }
        }
        StartPlan::Web(def) => {
            def.spec = bench.serving("caddy", def.admin_port, &[]);
            def.validate_command = bench.helper("noop", &[]);
        }
    }
}

struct Rig {
    bench: Shared,
    ports: PortAllocator,
    supervisor: Supervisor,
    controller: ServiceController,
    quitting: Arc<AtomicBool>,
}

fn rig(test: &str, os: Os) -> Rig {
    rig_with(test, os, stand_in)
}

fn rig_with(
    test: &str,
    os: Os,
    tune: impl Fn(&mut StartPlan, &Shared) + Send + Sync + 'static,
) -> Rig {
    let bench = private(test);
    let (supervisor, _events) = bench.supervisor();
    let ports = PortAllocator::new(bench.home.clone()).with_ranges(own_ranges());
    let quitting = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&quitting);
    // The hook gets its own handle on the bench (the same home and the same stand-in program).
    let for_hook = Shared {
        home: bench.home.clone(),
        stub: bench.stub.clone(),
    };
    let controller =
        ServiceController::new(bench.home.clone(), ports.clone(), supervisor.clone(), os)
            .with_quit_check(move || flag.load(Ordering::SeqCst))
            .with_plan_hook(move |plan| tune(plan, &for_hook));
    Rig {
        bench,
        ports,
        supervisor,
        controller,
        quitting,
    }
}

impl Rig {
    async fn start(&self, id: &str) -> luna_core::Result<()> {
        self.controller
            .start(id, &CancellationToken::new(), &|_| {})
            .await
    }

    fn state(&self, id: &str) -> Option<ServiceState> {
        self.supervisor.status(id).map(|s| s.state)
    }

    fn pid(&self, id: &str) -> Option<u32> {
        self.supervisor.status(id).and_then(|s| s.pid)
    }
}

fn workers(pool: &str, count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("{pool}-w{n}")).collect()
}

fn jobs_with_log() -> (JobManager, Arc<Mutex<Vec<JobEvent>>>) {
    let log: Arc<Mutex<Vec<JobEvent>>> = Arc::default();
    let sink = Arc::clone(&log);
    let jobs = JobManager::new(
        Handle::current(),
        move |event: JobEvent| sink.lock().unwrap().push(event),
        JobOptions::default(),
    );
    (jobs, log)
}

fn phases(log: &Mutex<Vec<JobEvent>>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for event in log.lock().unwrap().iter() {
        if let JobEvent::Progress(progress) = event
            && seen.last() != Some(&progress.phase)
        {
            seen.push(progress.phase.clone());
        }
    }
    seen
}

// ---------------------------------------------------------------------------------------------
// Which services a row is
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_row_id_names_its_services_on_each_system() {
    let windows = rig("control-members-win", Os::Windows);
    let linux = rig("control-members-lin", Os::Linux);

    assert_eq!(windows.controller.members("caddy").unwrap(), ["caddy"]);
    assert_eq!(
        windows.controller.members("mysql@8.4").unwrap(),
        ["mysql@8.4"]
    );
    assert_eq!(
        windows.controller.members("mariadb@11.8").unwrap(),
        ["mariadb@11.8"]
    );
    // A Windows pool is its workers, four until the pool was given another size.
    assert_eq!(
        windows.controller.members("php-fcgi@8.3.36").unwrap(),
        workers("php-fcgi@8.3.36", 4)
    );
    windows
        .ports
        .assign("php-fcgi@8.3.36", PortPurpose::PhpFastCgi { pool: 2 })
        .unwrap();
    assert_eq!(
        windows.controller.members("php-fcgi@8.3.36").unwrap(),
        workers("php-fcgi@8.3.36", 2)
    );
    assert_eq!(
        linux.controller.members("php-fpm@8.3.36").unwrap(),
        ["php-fpm@8.3.36"]
    );

    // Not services of this system, or not ids at all.
    for (rig, id) in [
        (&windows, "php-fpm@8.3.36"),
        (&linux, "php-fcgi@8.3.36"),
        (&linux, "php-fpm@banana"),
        (&linux, "mysql@8.4.13"),
        (&linux, "mysql@8"),
        (&linux, "mysql@"),
        (&linux, "mysql"),
        (&linux, "nginx"),
        (&linux, ""),
        (&linux, "caddy@2"),
    ] {
        let err = rig.controller.members(id).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound, "{id:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_row_of_the_page_is_a_service_the_controller_can_plan() {
    for os in [Os::Windows, Os::Linux] {
        let rig = rig("control-rows", os);
        let home = &rig.bench.home;
        install(home, "caddy", "2.11.7");
        install(home, "php", "8.3.36");
        install(home, "php", "8.4.2");
        install(home, "mysql", "8.4.6");
        install(home, "mysql", "8.4.13");
        install(home, "mariadb", "11.8.9");
        let installed = list_installed(home).unwrap();
        let rows = list_services(&rig.ports, &rig.supervisor, home, &installed, os).unwrap();
        assert_eq!(
            rows.len(),
            5,
            "{os:?}: caddy, two PHP, one MySQL line, one MariaDB line"
        );
        // Without the stand-in: each plan is built from the runtime its row says it runs from.
        let real =
            ServiceController::new(home.clone(), rig.ports.clone(), rig.supervisor.clone(), os);
        let program_of = |id: &str| match real.plan(id).unwrap() {
            StartPlan::Database(def) => def.spec.program,
            StartPlan::Php(def) => def.specs[0].program.clone(),
            StartPlan::Web(def) => def.spec.program,
        };
        let text = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
        assert!(
            text(program_of("mysql@8.4")).contains("8.4.13"),
            "newest patch of the line"
        );
        assert!(text(program_of("mariadb@11.8")).contains("11.8.9"));
        let pool = |version: &str| {
            if os == Os::Windows {
                format!("php-fcgi@{version}")
            } else {
                format!("php-fpm@{version}")
            }
        };
        assert!(
            text(program_of(&pool("8.3.36"))).contains("8.3.36"),
            "the exact PHP version"
        );
        assert!(text(program_of(&pool("8.4.2"))).contains("8.4.2"));
        assert!(text(program_of("caddy")).contains("2.11.7"));
        for row in rows {
            let plan = rig.controller.plan(&row.id).unwrap();
            let members: Vec<String> = row.members.iter().map(|m| m.id.clone()).collect();
            assert_eq!(plan.service_ids(), members, "{os:?} {}", row.id);
            assert_eq!(rig.controller.members(&row.id).unwrap(), members);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_whose_runtime_is_not_installed_cannot_be_planned() {
    let rig = rig("control-not-installed", Os::Linux);
    install(&rig.bench.home, "php", "8.3.36");
    for id in ["caddy", "mysql@8.4", "php-fpm@8.4.2"] {
        let err = rig.controller.plan(id).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound, "{id}");
        assert!(err.message.contains("not installed"), "{}", err.message);
    }
    // Planning wrote no config and took no port (a plan that fails changes nothing).
    assert!(rig.ports.all().unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// A database
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_database_is_set_up_on_its_first_start_and_not_on_the_next() {
    let rig = rig("control-db-first", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let seen = Mutex::new(Vec::new());
    rig.controller
        .start("mariadb@11.8", &CancellationToken::new(), &|p| {
            seen.lock().unwrap().push(p.to_owned());
        })
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["setup", "starting"]);
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Running));
    assert!(
        rig.bench
            .home
            .data_dir()
            .join("mariadb-11.8")
            .join("mysql")
            .exists()
    );
    let port = rig.ports.assignment("mariadb@11.8").unwrap().unwrap().port;
    assert!(port_open(port), "it answers on the port it was given");
    assert_eq!(launches(&setup_counter(&rig.bench)), 1);

    // Stopped and started again: the data folder is there, so no setup and no `setup` phase.
    rig.controller.stop("mariadb@11.8").await.unwrap();
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Stopped));
    let seen = Mutex::new(Vec::new());
    rig.controller
        .start("mariadb@11.8", &CancellationToken::new(), &|p| {
            seen.lock().unwrap().push(p.to_owned());
        })
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["starting"]);
    assert_eq!(launches(&setup_counter(&rig.bench)), 1, "set up once only");
    rig.controller.stop("mariadb@11.8").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_setup_starts_nothing_and_says_what_the_setup_printed() {
    let rig = rig_with("control-db-init-fails", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Database(def) = plan {
            let data = def.data_dir.display().to_string();
            def.init = bench.helper("init-fail", &[("LUNA_STUB_DATADIR", data.as_str())]);
        }
    });
    install(&rig.bench.home, "mysql", "8.4.13");
    let err = rig.start("mysql@8.4").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::DatabaseInitFailed);
    let tail = err.details.as_ref().unwrap()["tail"].to_string();
    assert!(
        tail.contains("boom: cannot create the system tables"),
        "{tail}"
    );
    assert_eq!(rig.state("mysql@8.4"), None, "no service was started");
    assert!(
        !rig.bench.home.data_dir().join("mysql-8.4").exists(),
        "the half-made data folder is removed, so the next try starts clean"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_the_setup_leaves_no_data_folder_and_no_service() {
    let rig = rig("control-db-cancel-setup", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = rig
        .controller
        .start("mariadb@11.8", &cancel, &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(rig.state("mariadb@11.8"), None);
    assert!(!rig.bench.home.data_dir().join("mariadb-11.8").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_it_waits_for_the_database_stops_it() {
    let rig = rig_with("control-db-cancel-wait", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Database(def) = plan {
            // Listens only after five seconds: the start is still waiting when the cancel comes.
            def.spec = bench.serving(
                &def.spec.id.clone(),
                def.port,
                &[("LUNA_STUB_DELAY_MS", "5000")],
            );
        }
    });
    install(&rig.bench.home, "mariadb", "11.8.9");
    // Already set up, so the start goes straight to waiting.
    let data = rig.bench.home.data_dir().join("mariadb-11.8");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("ibdata1"), "x").unwrap();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let watched = rig.supervisor.clone();
    tokio::spawn(async move {
        // The process exists once the supervisor lists the service as starting.
        for _ in 0..400 {
            if watched.status("mariadb@11.8").map(|s| s.state) == Some(ServiceState::Starting) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        trigger.cancel();
    });
    let err = rig
        .controller
        .start("mariadb@11.8", &cancel, &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Stopped));
    let port = rig.ports.assignment("mariadb@11.8").unwrap().unwrap().port;
    wait_until_free(port).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_never_becomes_ready_reports_why_and_keeps_the_error_on_the_service() {
    let rig = rig_with("control-db-fails", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Database(def) = plan {
            // Leaves with exit code 2 before it listens.
            def.spec = bench.serving(
                &def.spec.id.clone(),
                def.port,
                &[("LUNA_STUB_FAIL_FROM", "1")],
            );
        }
    });
    install(&rig.bench.home, "mariadb", "11.8.9");
    let err = rig.start("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ServiceStartFailed);
    let status = rig.supervisor.status("mariadb@11.8").unwrap();
    assert_eq!(status.state, ServiceState::Error);
    assert_eq!(
        status.error.as_ref().map(|e| e.code),
        Some(ErrorCode::ServiceStartFailed)
    );

    // Stop clears the failure, and the service can be started again.
    rig.controller.stop("mariadb@11.8").await.unwrap();
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Stopped));
}

// ---------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_port_is_refused_with_who_has_it_and_another_port_can_be_used() {
    let rig = rig("control-port", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let port = rig
        .ports
        .assign("mariadb@11.8", PortPurpose::Database)
        .unwrap()
        .port;
    let holder = TcpListener::bind(("127.0.0.1", port)).unwrap();

    let err = rig.start("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse);
    let details = err.details.as_ref().unwrap();
    assert_eq!(details["port"], port);
    assert!(
        details["pid"].as_u64().is_some(),
        "it names the holder: {details}"
    );
    assert_eq!(rig.state("mariadb@11.8"), None, "nothing was started");

    // "Use another port": the next free one, and the service runs on it.
    let (jobs, _log) = jobs_with_log();
    let started = rig
        .controller
        .reassign_and_start(&jobs, "mariadb@11.8")
        .unwrap();
    let info = jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Succeeded, "{:?}", info.error);
    let moved = rig.ports.assignment("mariadb@11.8").unwrap().unwrap().port;
    assert_ne!(moved, port);
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Running));
    assert!(port_open(moved));
    drop(holder);

    // It runs now, so it cannot be moved from under itself.
    let err = rig
        .controller
        .reassign_and_start(&jobs, "mariadb@11.8")
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    assert_eq!(
        rig.ports.assignment("mariadb@11.8").unwrap().unwrap().port,
        moved
    );
    rig.controller.stop("mariadb@11.8").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn caddys_admin_port_has_no_other_port_to_offer() {
    let rig = rig("control-caddy-port", Os::Linux);
    install(&rig.bench.home, "caddy", "2.11.7");
    let admin = rig
        .ports
        .assign("caddy", PortPurpose::CaddyAdmin)
        .unwrap()
        .port;
    let _holder = TcpListener::bind(("127.0.0.1", admin)).unwrap();

    let err = rig.start("caddy").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse);
    assert_eq!(err.details.as_ref().unwrap()["port"], admin);

    let (jobs, _log) = jobs_with_log();
    let err = rig
        .controller
        .reassign_and_start(&jobs, "caddy")
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse, "{}", err.message);
    assert_eq!(rig.state("caddy"), None);
}

// ---------------------------------------------------------------------------------------------
// A PHP pool
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_windows_pool_starts_and_stops_as_one() {
    let rig = rig("control-pool", Os::Windows);
    install(&rig.bench.home, "php", "8.3.36");
    let pool = "php-fcgi@8.3.36";
    let members = workers(pool, 4);

    rig.start(pool).await.unwrap();
    let before: Vec<Option<u32>> = members.iter().map(|m| rig.pid(m)).collect();
    for member in &members {
        assert_eq!(rig.state(member), Some(ServiceState::Running), "{member}");
    }
    let ports = rig.ports.assignment(pool).unwrap().unwrap().ports();
    assert_eq!(ports.len(), 4);
    assert!(ports.iter().all(|&p| port_open(p)));

    // Starting what runs is refused and leaves it alone.
    let err = rig.start(pool).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    let same: Vec<Option<u32>> = members.iter().map(|m| rig.pid(m)).collect();
    assert_eq!(before, same);

    // A restart replaces every process.
    rig.controller
        .restart(pool, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    for (member, old) in members.iter().zip(&before) {
        assert_eq!(rig.state(member), Some(ServiceState::Running), "{member}");
        assert_ne!(&rig.pid(member), old, "{member} is a new process");
    }

    rig.controller.stop(pool).await.unwrap();
    for member in &members {
        assert_eq!(rig.state(member), Some(ServiceState::Stopped), "{member}");
    }
    for port in ports {
        wait_until_free(port).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pool_with_a_taken_port_starts_no_worker_at_all() {
    let rig = rig("control-pool-port", Os::Windows);
    install(&rig.bench.home, "php", "8.3.36");
    let pool = "php-fcgi@8.3.36";
    let ports = rig
        .ports
        .assign(pool, PortPurpose::PhpFastCgi { pool: 4 })
        .unwrap()
        .ports();
    // The third worker's port is taken.
    let _holder = TcpListener::bind(("127.0.0.1", ports[2])).unwrap();

    let err = rig.start(pool).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse);
    assert_eq!(err.details.as_ref().unwrap()["port"], ports[2]);
    for member in workers(pool, 4) {
        assert_eq!(
            rig.state(&member),
            None,
            "{member} must not have been started"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_that_fails_takes_the_started_ones_down_with_it() {
    let rig = rig_with("control-pool-rollback", Os::Windows, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Php(def) = plan {
            // The third worker leaves at once.
            let spec = &mut def.specs[2];
            *spec = bench.serving(
                &spec.id.clone(),
                spec.ports[0],
                &[("LUNA_STUB_FAIL_FROM", "1")],
            );
        }
    });
    install(&rig.bench.home, "php", "8.3.36");
    let pool = "php-fcgi@8.3.36";

    let err = rig.start(pool).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ServiceStartFailed);
    let w = workers(pool, 4);
    for ok in [&w[0], &w[1], &w[3]] {
        assert_eq!(
            rig.state(ok),
            Some(ServiceState::Stopped),
            "{ok} was started, then stopped again"
        );
    }
    assert_eq!(
        rig.state(&w[2]),
        Some(ServiceState::Error),
        "the one that failed keeps its error"
    );

    // The page's Stop clears it.
    rig.controller.stop(pool).await.unwrap();
    for member in &w {
        assert_eq!(rig.state(member), Some(ServiceState::Stopped), "{member}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn php_fpm_is_one_service() {
    let rig = rig("control-fpm", Os::Linux);
    install(&rig.bench.home, "php", "8.4.2");
    rig.start("php-fpm@8.4.2").await.unwrap();
    assert_eq!(rig.state("php-fpm@8.4.2"), Some(ServiceState::Running));
    assert_eq!(rig.supervisor.list().len(), 1);
    rig.controller.stop("php-fpm@8.4.2").await.unwrap();
    assert_eq!(rig.state("php-fpm@8.4.2"), Some(ServiceState::Stopped));
}

// ---------------------------------------------------------------------------------------------
// Caddy
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn caddy_starts_with_no_sites_and_a_generated_caddyfile() {
    let rig = rig("control-caddy", Os::Linux);
    install(&rig.bench.home, "caddy", "2.11.7");
    rig.start("caddy").await.unwrap();
    assert_eq!(rig.state("caddy"), Some(ServiceState::Running));
    let caddyfile =
        fs::read_to_string(rig.bench.home.config_dir().join("caddy").join("Caddyfile")).unwrap();
    assert!(caddyfile.contains("admin 127.0.0.1:"), "{caddyfile}");
    assert!(!caddyfile.contains("http://"), "no sites yet: {caddyfile}");
    rig.controller.stop("caddy").await.unwrap();
    assert_eq!(rig.state("caddy"), Some(ServiceState::Stopped));
}

#[tokio::test(flavor = "multi_thread")]
async fn caddy_is_not_started_when_it_rejects_its_caddyfile() {
    let rig = rig_with("control-caddy-invalid", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Web(def) = plan {
            def.validate_command = bench.helper("exit", &[("LUNA_STUB_EXIT_CODE", "1")]);
        }
    });
    install(&rig.bench.home, "caddy", "2.11.7");
    let err = rig.start("caddy").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert_eq!(
        rig.state("caddy"),
        None,
        "an invalid Caddyfile starts nothing"
    );
}

// ---------------------------------------------------------------------------------------------
// Stop, restart, busy, quit
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn stopping_what_never_ran_does_nothing_and_an_unknown_id_is_not_found() {
    let rig = rig("control-stop-idle", Os::Windows);
    rig.controller.stop("caddy").await.unwrap();
    rig.controller.stop("php-fcgi@8.3.36").await.unwrap();
    rig.controller.stop("mysql@8.4").await.unwrap();
    assert!(rig.supervisor.list().is_empty());
    let err = rig.controller.stop("nginx").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_stops_then_starts_and_starts_what_is_stopped() {
    let rig = rig("control-restart", Os::Linux);
    install(&rig.bench.home, "caddy", "2.11.7");
    // Stopped: a restart is a start.
    rig.controller
        .restart("caddy", &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    let first = rig.pid("caddy").unwrap();
    let seen = Mutex::new(Vec::new());
    rig.controller
        .restart("caddy", &CancellationToken::new(), &|p| {
            seen.lock().unwrap().push(p.to_owned());
        })
        .await
        .unwrap();
    assert_eq!(rig.state("caddy"), Some(ServiceState::Running));
    assert_ne!(rig.pid("caddy").unwrap(), first, "a new process");
    assert_eq!(
        seen.lock().unwrap().first().map(String::as_str),
        Some("stopping")
    );
    rig.controller.stop("caddy").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_service_is_refused_with_busy_and_its_config_is_left_alone() {
    let rig = rig("control-busy", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    rig.start("mariadb@11.8").await.unwrap();
    let config = rig
        .bench
        .home
        .config_dir()
        .join("mariadb")
        .join("11.8")
        .join("my.cnf");
    fs::write(&config, "# touched by the test\n").unwrap();
    let pid = rig.pid("mariadb@11.8");

    let err = rig.start("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    assert_eq!(rig.pid("mariadb@11.8"), pid);
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "# touched by the test\n",
        "the config of a running database is not rewritten"
    );
    rig.controller.stop("mariadb@11.8").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_started_while_the_app_quits_but_stopping_still_works() {
    let rig = rig("control-quit", Os::Linux);
    install(&rig.bench.home, "caddy", "2.11.7");
    rig.start("caddy").await.unwrap();

    rig.quitting.store(true, Ordering::SeqCst);
    let (jobs, _log) = jobs_with_log();
    for action in [ServiceAction::Start, ServiceAction::Restart] {
        let err = rig.controller.run(&jobs, "caddy", action).unwrap_err();
        assert_eq!(err.code, ErrorCode::Busy, "{action:?}");
        assert!(err.message.contains("quitting"), "{}", err.message);
    }
    // A service that is not running: refused because of the quit, before anything is built or started.
    install(&rig.bench.home, "mariadb", "11.8.9");
    let err = rig.start("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    assert!(err.message.contains("quitting"), "{}", err.message);
    assert_eq!(rig.state("mariadb@11.8"), None);
    assert!(
        !rig.bench.home.config_dir().join("mariadb").exists(),
        "no config was written for a start that was refused"
    );
    let err = rig
        .controller
        .reassign_and_start(&jobs, "caddy")
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);

    // Stopping is what the quit does anyway.
    let started = rig
        .controller
        .run(&jobs, "caddy", ServiceAction::Stop)
        .unwrap();
    assert_eq!(
        jobs.wait(&started.id).await.unwrap().state,
        JobState::Succeeded
    );
    assert_eq!(rig.state("caddy"), Some(ServiceState::Stopped));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quit_that_begins_during_the_setup_stops_the_start_before_a_process_exists() {
    let mut rig = rig("control-quit-mid", Os::Linux);
    let quitting = Arc::clone(&rig.quitting);
    let bench = Shared {
        home: rig.bench.home.clone(),
        stub: rig.bench.stub.clone(),
    };
    // The plan is built and the setup is about to run: the app begins to quit.
    rig.controller = rig.controller.clone().with_plan_hook(move |plan| {
        stand_in(plan, &bench);
        quitting.store(true, Ordering::SeqCst);
    });
    install(&rig.bench.home, "mariadb", "11.8.9");
    // The first look at the flag (before the plan) still passes, so the plan is built and the hook flips it.
    let err = rig.start("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    assert!(err.message.contains("quitting"), "{}", err.message);
    assert_eq!(rig.state("mariadb@11.8"), None, "no process was started");
}

// ---------------------------------------------------------------------------------------------
// As a job
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_action_is_a_job_named_by_its_service_with_phases_and_a_result() {
    let rig = rig("control-job", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let (jobs, log) = jobs_with_log();

    let started = rig
        .controller
        .run(&jobs, "mariadb@11.8", ServiceAction::Start)
        .unwrap();
    assert!(!started.already_running);
    let running = jobs.get(&started.id).unwrap();
    assert_eq!(running.key, job_key("mariadb@11.8"));
    assert_eq!(running.key, "service:mariadb@11.8");
    assert_eq!(running.label, "Start mariadb@11.8");
    let info = jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Succeeded, "{:?}", info.error);
    // A job begins in the phase `starting`; what the controller reports follows.
    let seen = phases(&log);
    assert_eq!(seen[seen.len() - 2..], ["setup", "starting"], "{seen:?}");
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Running));

    let stopped = rig
        .controller
        .run(&jobs, "mariadb@11.8", ServiceAction::Stop)
        .unwrap();
    assert_eq!(
        jobs.get(&stopped.id).map(|j| j.label),
        Some("Stop mariadb@11.8".to_owned())
    );
    assert_eq!(
        jobs.wait(&stopped.id).await.unwrap().state,
        JobState::Succeeded
    );
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Stopped));

    // A failure is the job's error, with the code and details the page maps to text.
    let port = rig.ports.assignment("mariadb@11.8").unwrap().unwrap().port;
    let holder = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let failed = rig
        .controller
        .run(&jobs, "mariadb@11.8", ServiceAction::Start)
        .unwrap();
    let info = jobs.wait(&failed.id).await.unwrap();
    assert_eq!(info.state, JobState::Failed);
    let error = info.error.unwrap();
    assert_eq!(error.code, ErrorCode::PortInUse);
    assert_eq!(error.details.unwrap()["port"], port);
    drop(holder);

    let err = rig
        .controller
        .run(&jobs, "nginx", ServiceAction::Start)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_action_joins_the_running_job_and_another_one_is_busy() {
    let rig = rig_with("control-job-busy", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Web(def) = plan {
            def.spec = bench.serving("caddy", def.admin_port, &[("LUNA_STUB_DELAY_MS", "1500")]);
        }
    });
    install(&rig.bench.home, "caddy", "2.11.7");
    let (jobs, _log) = jobs_with_log();

    let first = rig
        .controller
        .run(&jobs, "caddy", ServiceAction::Start)
        .unwrap();
    let again = rig
        .controller
        .run(&jobs, "caddy", ServiceAction::Start)
        .unwrap();
    assert!(again.already_running);
    assert_eq!(again.id, first.id);
    let err = rig
        .controller
        .run(&jobs, "caddy", ServiceAction::Stop)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    let err = rig
        .controller
        .run(&jobs, "caddy", ServiceAction::Restart)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);

    assert_eq!(
        jobs.wait(&first.id).await.unwrap().state,
        JobState::Succeeded
    );
    rig.controller.stop("caddy").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_the_job_of_a_start_stops_the_service_and_ends_as_cancelled() {
    let rig = rig_with("control-job-cancel", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Web(def) = plan {
            def.spec = bench.serving("caddy", def.admin_port, &[("LUNA_STUB_DELAY_MS", "8000")]);
        }
    });
    install(&rig.bench.home, "caddy", "2.11.7");
    let (jobs, _log) = jobs_with_log();
    let started = rig
        .controller
        .run(&jobs, "caddy", ServiceAction::Start)
        .unwrap();
    // Wait until the process exists (the supervisor knows the service), then cancel.
    for _ in 0..200 {
        if rig.state("caddy") == Some(ServiceState::Starting) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(rig.state("caddy"), Some(ServiceState::Starting));
    assert!(jobs.cancel(&started.id).unwrap());
    let info = jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Cancelled, "{:?}", info.error);
    assert_eq!(info.error.map(|e| e.code), Some(ErrorCode::Cancelled));
    assert_eq!(rig.state("caddy"), Some(ServiceState::Stopped));
}

// ---------------------------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------------------------

/// Wait until `id` has printed at least `count` lines of its `output` source (the pump reads every tick).
async fn wait_for_output(
    rig: &Rig,
    id: &str,
    count: usize,
) -> Vec<luna_core::services::ServiceLogs> {
    for _ in 0..400 {
        let logs = rig.controller.logs(id, 1_000).unwrap();
        let have = logs
            .iter()
            .map(|service| {
                service
                    .sources
                    .iter()
                    .filter(|source| source.source == "output")
                    .map(|source| source.lines.len())
                    .sum::<usize>()
            })
            .min()
            .unwrap_or(0);
        if have >= count {
            return logs;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{id} did not print {count} lines in ten seconds");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_log_of_a_row_is_its_services_sources_with_the_last_lines_in_order() {
    let rig = rig_with("control-logs", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Web(def) = plan {
            def.spec = bench.serving("caddy", def.admin_port, &[("LUNA_STUB_OUT_COUNT", "5")]);
        }
    });
    install(&rig.bench.home, "caddy", "2.11.7");
    rig.start("caddy").await.unwrap();

    let logs = wait_for_output(&rig, "caddy", 6).await;

    assert_eq!(logs.len(), 1, "one service");
    assert_eq!(logs[0].service, "caddy");
    assert_eq!(
        logs[0].sources[0].source, "output",
        "the output comes first"
    );
    let text: Vec<&str> = logs[0].sources[0]
        .lines
        .iter()
        .map(|l| l.line.as_str())
        .collect();
    let printed: Vec<&str> = text
        .iter()
        .copied()
        .filter(|l| l.starts_with("out "))
        .collect();
    assert_eq!(
        printed,
        ["out 0", "out 1", "out 2", "out 3", "out 4"],
        "{text:?}"
    );
    assert!(
        text[0].contains("[lunaRun]"),
        "lunaRun writes the first line itself: {text:?}"
    );

    // Only the last lines, still oldest first; a huge request is capped, not an error.
    let two = rig.controller.logs("caddy", 2).unwrap();
    assert_eq!(two[0].sources[0].lines.len(), 2);
    assert_eq!(two[0].sources[0].lines[1].line, text[text.len() - 1]);
    assert!(rig.controller.logs("caddy", usize::MAX).is_ok());
    assert_eq!(
        rig.controller.logs("caddy", 0).unwrap()[0].sources[0]
            .lines
            .len(),
        0
    );

    // A stopped service keeps what it printed until it is started again.
    rig.controller.stop("caddy").await.unwrap();
    let after = rig.controller.logs("caddy", 1_000).unwrap();
    assert!(after[0].sources[0].lines.iter().any(|l| l.line == "out 4"));
    assert!(
        after[0].sources[0]
            .lines
            .iter()
            .any(|l| l.line.contains("stopped"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_windows_pool_has_one_log_per_worker_and_an_unknown_id_is_not_found() {
    let rig = rig("control-logs-pool", Os::Windows);
    install(&rig.bench.home, "php", "8.3.36");
    let pool = "php-fcgi@8.3.36";

    // Never started: the row's services are named, with nothing to show yet.
    let idle = rig.controller.logs(pool, 100).unwrap();
    assert_eq!(
        idle.iter().map(|l| l.service.clone()).collect::<Vec<_>>(),
        workers(pool, 4)
    );
    assert!(idle.iter().all(|l| l.sources.is_empty()));

    rig.start(pool).await.unwrap();
    let logs = wait_for_output(&rig, pool, 1).await;
    assert_eq!(logs.len(), 4);
    for (log, worker) in logs.iter().zip(workers(pool, 4)) {
        assert_eq!(log.service, worker);
        assert_eq!(log.sources[0].source, "output");
    }
    rig.controller.stop(pool).await.unwrap();

    assert_eq!(
        rig.controller.logs("nginx", 10).unwrap_err().code,
        ErrorCode::NotFound
    );
}

// ---------------------------------------------------------------------------------------------
// Leftovers: adopt and stop
// ---------------------------------------------------------------------------------------------

/// A service an earlier run left behind: the stand-in running on the port the service is given, with a PID file.
async fn leave_behind(rig: &Rig, id: &str, port: u16) -> Detached {
    let left = detached(&rig.bench, port, &[]);
    write_pid_file(&rig.bench, id, left.pid, &rig.bench.stub, Some(port));
    for _ in 0..400 {
        if port_open(port) {
            return left;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the left-over stand-in never listened on {port}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_leftover_is_adopted_and_then_stopped_like_any_other_service() {
    let rig = rig("control-adopt", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let port = rig
        .ports
        .assign("mariadb@11.8", PortPurpose::Database)
        .unwrap()
        .port;
    let left = leave_behind(&rig, "mariadb@11.8", port).await;
    let found = rig.supervisor.scan_leftovers();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "mariadb@11.8");

    rig.controller.adopt("mariadb@11.8").await.unwrap();

    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Running));
    assert_eq!(
        rig.pid("mariadb@11.8"),
        Some(left.pid),
        "the same process, now tracked"
    );
    // From now on it is an ordinary service: a start is refused as busy, and a stop ends it.
    assert_eq!(
        rig.start("mariadb@11.8").await.unwrap_err().code,
        ErrorCode::Busy
    );
    rig.controller.stop("mariadb@11.8").await.unwrap();
    assert_eq!(rig.state("mariadb@11.8"), Some(ServiceState::Stopped));
    assert!(
        wait_gone(left.pid, Duration::from_secs(10)).await,
        "the process is gone"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_of_a_pool_is_adopted_through_its_pool() {
    let rig = rig("control-adopt-worker", Os::Windows);
    install(&rig.bench.home, "php", "8.3.36");
    let pool = "php-fcgi@8.3.36";
    let ports = rig
        .ports
        .assign(pool, PortPurpose::PhpFastCgi { pool: 4 })
        .unwrap()
        .ports();
    let worker = "php-fcgi@8.3.36-w2";
    let left = leave_behind(&rig, worker, ports[1]).await;

    rig.controller.adopt(worker).await.unwrap();

    assert_eq!(rig.state(worker), Some(ServiceState::Running));
    assert_eq!(rig.pid(worker), Some(left.pid));
    rig.controller.stop(pool).await.unwrap();
    assert!(wait_gone(left.pid, Duration::from_secs(10)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn adopting_what_is_not_there_or_not_installed_or_not_the_same_program_changes_nothing() {
    // Nothing left running.
    let rig = rig("control-adopt-refused", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let err = rig.controller.adopt("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(err.message.contains("left running"), "{}", err.message);
    assert_eq!(rig.state("mariadb@11.8"), None);

    // Its version is not installed any more: it can only be stopped.
    let err = rig.controller.adopt("mysql@8.4").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(err.message.contains("not installed"), "{}", err.message);

    // Not a service of this system.
    assert_eq!(
        rig.controller.adopt("nginx").await.unwrap_err().code,
        ErrorCode::NotFound
    );

    // What runs is not the program the definition names: it is left alone.
    let other = rig_with("control-adopt-other", Os::Linux, |plan, bench| {
        stand_in(plan, bench);
        if let StartPlan::Database(def) = plan {
            let copy = bench.stub.with_file_name("another-program");
            fs::copy(&bench.stub, &copy).unwrap();
            def.spec.program = copy;
        }
    });
    install(&other.bench.home, "mariadb", "11.8.9");
    let port = other
        .ports
        .assign("mariadb@11.8", PortPurpose::Database)
        .unwrap()
        .port;
    let left = leave_behind(&other, "mariadb@11.8", port).await;
    let err = other.controller.adopt("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPath, "{}", err.message);
    assert!(
        luna_core::services::process_alive(left.pid),
        "the process was not touched"
    );
    assert!(
        pid_file(&other.bench, "mariadb@11.8").exists(),
        "nor its PID file"
    );
    assert_eq!(other.state("mariadb@11.8"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_leftover_is_stopped_and_forgotten_and_nothing_is_adopted_while_quitting() {
    let rig = rig("control-stop-leftover", Os::Linux);
    install(&rig.bench.home, "mariadb", "11.8.9");
    let port = rig
        .ports
        .assign("mariadb@11.8", PortPurpose::Database)
        .unwrap()
        .port;
    let left = leave_behind(&rig, "mariadb@11.8", port).await;
    assert_eq!(rig.supervisor.scan_leftovers().len(), 1);

    // The quit has begun: adopting is refused (a service adopted now would outlive the orderly stop), stopping is not.
    rig.quitting.store(true, Ordering::SeqCst);
    let err = rig.controller.adopt("mariadb@11.8").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy);
    assert!(luna_core::services::process_alive(left.pid));

    rig.controller.stop_leftover("mariadb@11.8").await.unwrap();

    assert!(
        wait_gone(left.pid, Duration::from_secs(15)).await,
        "the process was ended"
    );
    assert!(
        !pid_file(&rig.bench, "mariadb@11.8").exists(),
        "and its PID file is gone"
    );
    let err = rig
        .controller
        .stop_leftover("mariadb@11.8")
        .await
        .unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::NotFound,
        "a second time there is nothing left"
    );
    assert_eq!(
        rig.controller
            .stop_leftover("nginx")
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}
