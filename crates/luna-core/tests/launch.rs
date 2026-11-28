//! Starting and stopping a project (D-039): that a start brings up its database, its PHP pool and Caddy with the project
//! in the Caddyfile, each only when it is not already up; that it is all or none; and that a stop takes the project out
//! of Caddy and stops only what the start began and no other started project needs. The runtimes are folders with
//! receipts, and the programs the services would run are replaced by the stand-in service (the real ones are run by
//! the ignored `sites_online` test and the real app window). Every data folder and project folder has a space and a
//! non-ASCII letter in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use luna_core::diagnostics::{Arch, Os};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::launch::{ProjectAction, ProjectLauncher, job_key};
use luna_core::projects::{DetectionData, Project, ProjectStore, Surroundings};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::{InstallReceipt, php_config_paths};
use luna_core::services::{
    PortAllocator, PortRanges, ServiceController, ServiceState, StartPlan, Supervisor, pool_id,
};
use luna_core::sites::{SiteManager, SiteState};
use luna_core::{ErrorCode, LunaHome};
use stub_service::*;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

struct Rig {
    bench: Shared,
    store: ProjectStore,
    ports: PortAllocator,
    supervisor: Supervisor,
    sites: SiteManager,
    controller: ServiceController,
    launcher: ProjectLauncher,
    os: Os,
    reload_runs: PathBuf,
    /// While set, the stand-in `caddy validate` fails.
    validate_fails: Arc<AtomicBool>,
}

fn rig(test: &str, os: Os) -> Rig {
    let bench = private(test);
    let (supervisor, _events) = bench.supervisor();
    let ports = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        site: free_block(24),
        php_fastcgi: free_block(16),
        caddy_admin: free_block(1),
        database: free_block(8),
    });
    let store = ProjectStore::open(&bench.home).unwrap();
    let scratch = bench.scratch(test);
    let reload_runs = scratch.join("reload-runs");
    let validate_fails = Arc::new(AtomicBool::new(false));
    let shared = |bench: &Shared| Shared {
        home: bench.home.clone(),
        stub: bench.stub.clone(),
    };

    // Caddy's validate and reload, for the site manager (a running Caddy is given a change).
    let (site_bench, failing, runs) = (
        shared(&bench),
        Arc::clone(&validate_fails),
        reload_runs.clone(),
    );
    let data_dir = scratch.join("stub-data");
    let sites = SiteManager::new(
        bench.home.clone(),
        store.clone(),
        Arc::new(DetectionData::bundled().unwrap()),
        RegistryHolder::with_bundled().unwrap(),
        ports.clone(),
        supervisor.clone(),
        os,
    )
    .with_definition_hook(move |definition| {
        definition.validate_command = if failing.load(Ordering::SeqCst) {
            site_bench.helper("exit", &[("LUNA_STUB_EXIT_CODE", "1")])
        } else {
            site_bench.helper("noop", &[])
        };
        definition.reload_command = site_bench.helper(
            "init-ok",
            &[
                ("LUNA_STUB_DATADIR", data_dir.to_str().unwrap()),
                ("LUNA_STUB_COUNTER", runs.to_str().unwrap()),
            ],
        );
    });

    // The controller, with the stand-in in place of every real program.
    let (hook_bench, setup_counter, source) =
        (shared(&bench), scratch.join("setup-runs"), sites.clone());
    let controller =
        ServiceController::new(bench.home.clone(), ports.clone(), supervisor.clone(), os)
            .with_sites(move || source.sites())
            .with_plan_hook(move |plan| match plan {
                StartPlan::Database(def) => {
                    def.spec = hook_bench.serving(&def.spec.id.clone(), def.port, &[]);
                    let data = def.data_dir.display().to_string();
                    def.init = hook_bench.helper(
                        "init-ok",
                        &[
                            ("LUNA_STUB_DATADIR", data.as_str()),
                            ("LUNA_STUB_COUNTER", setup_counter.to_str().unwrap()),
                        ],
                    );
                }
                StartPlan::Php(def) => {
                    for spec in &mut def.specs {
                        *spec = hook_bench.serving(&spec.id.clone(), spec.ports[0], &[]);
                    }
                }
                StartPlan::Web(def) => {
                    // The stand-in listens on the admin port; the ports to check before the start stay the real ones.
                    let checked = def.spec.ports.clone();
                    def.spec = hook_bench.serving("caddy", def.admin_port, &[]);
                    def.spec.ports = checked;
                    def.validate_command = hook_bench.helper("noop", &[]);
                }
            });
    let launcher = ProjectLauncher::new(
        store.clone(),
        sites.clone(),
        controller.clone(),
        ports.clone(),
        os,
    );
    Rig {
        bench,
        store,
        ports,
        supervisor,
        sites,
        controller,
        launcher,
        os,
        reload_runs,
        validate_fails,
    }
}

fn install(home: &LunaHome, runtime: &str, version: &str) {
    let dir = home.runtime_dir(runtime, version).unwrap();
    fs::create_dir_all(dir.join("bin")).unwrap();
    InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
        .write(&dir)
        .unwrap();
    if runtime == "php" {
        // The php.ini the installer generates: a pool refuses to start without it.
        let paths = php_config_paths(home, version).unwrap();
        fs::create_dir_all(&paths.dir).unwrap();
        fs::write(
            &paths.ini,
            "; generated at install
",
        )
        .unwrap();
    }
}

impl Rig {
    fn project(&self, name: &str, env: Option<&str>) -> Project {
        let dir = self.bench.scratch("projects").join(format!("mon {name} é"));
        fs::create_dir_all(dir.join("public")).unwrap();
        fs::write(dir.join("public").join("index.php"), "<?php").unwrap();
        fs::write(dir.join("artisan"), "").unwrap();
        fs::write(dir.join("composer.json"), r#"{"require":{"php":"^8.2"}}"#).unwrap();
        if let Some(env) = env {
            fs::write(dir.join(".env"), env).unwrap();
        }
        let around = Surroundings {
            installed: &[],
            available: &[],
            os: self.os,
            defaults: &luna_core::settings::RuntimeDefaults::NONE,
        };
        self.store
            .add(&dir, &DetectionData::bundled().unwrap(), &around)
            .unwrap()
            .project
    }

    async fn start(&self, id: &str) -> luna_core::Result<()> {
        self.launcher
            .start(id, &CancellationToken::new(), &|_| {})
            .await
    }

    fn state(&self, id: &str) -> Option<ServiceState> {
        self.supervisor.status(id).map(|s| s.state)
    }

    fn entry(&self, project: &Project) -> luna_core::sites::SiteEntry {
        self.sites
            .plan()
            .unwrap()
            .entries
            .into_iter()
            .find(|e| e.project_id == project.id)
            .unwrap()
    }

    fn pool(&self) -> String {
        pool_id(self.os, "8.3.14")
    }

    /// Is the pool of PHP 8.3.14 up? On Windows that is every worker.
    fn pool_up(&self) -> bool {
        luna_core::services::pool_is_running(&self.ports, &self.supervisor, self.os, "8.3.14")
            .unwrap()
    }

    fn caddyfile(&self) -> String {
        fs::read_to_string(self.bench.home.config_dir().join("caddy").join("Caddyfile"))
            .unwrap_or_default()
    }

    /// The database line the registry suggests for a MySQL `.env`: the line the fake MySQL has to be installed as.
    fn install_database_for(&self, project: &Project) {
        let (_, resolution) = self.sites.evaluate(project).unwrap();
        let line = resolution.database.unwrap().line.unwrap();
        install(&self.bench.home, "mysql", &format!("{line}.13"));
    }

    async fn stop_everything(&self) {
        for row in ["caddy".to_owned(), self.pool(), "mysql@x".to_owned()] {
            let _ = self.controller.stop(&row).await;
        }
        for status in self.supervisor.list() {
            let _ = self.supervisor.stop(&status.id).await;
        }
    }
}

fn runs(path: &Path) -> usize {
    launches(path)
}

const ENV_MYSQL: &str = "DB_CONNECTION=mysql\nDB_DATABASE=shop\nDB_PASSWORD=never-read\n";

// ---------------------------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn starting_a_project_brings_up_its_database_its_pool_and_caddy_with_the_project_in_the_caddyfile()
 {
    let rig = rig("launch-start", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", Some(ENV_MYSQL));
    rig.install_database_for(&shop);
    let database = rig.entry(&shop).database_service.unwrap();
    assert!(database.starts_with("mysql@"), "{database}");
    assert!(!rig.entry(&shop).running);

    rig.start(&shop.id).await.unwrap();

    assert_eq!(rig.state(&database), Some(ServiceState::Running));
    assert!(rig.pool_up(), "the pool of its PHP runs");
    assert_eq!(rig.state("caddy"), Some(ServiceState::Running));
    let entry = rig.entry(&shop);
    assert!(entry.served && entry.running, "{entry:?}");
    let site = entry.address.clone().unwrap();
    let port = site.rsplit(':').next().unwrap();
    assert!(
        rig.caddyfile()
            .contains(&format!("http://localhost:{port}"))
    );
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_that_expects_no_database_starts_only_its_pool_and_caddy() {
    let rig = rig("launch-nodb", Os::Linux);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    install(&rig.bench.home, "mysql", "8.4.13");
    let blog = rig.project("blog", None);

    rig.start(&blog.id).await.unwrap();

    assert_eq!(rig.state(&rig.pool()), Some(ServiceState::Running));
    assert_eq!(rig.state("caddy"), Some(ServiceState::Running));
    assert!(
        rig.supervisor.status("mysql@8.4").is_none(),
        "no database was started for a project that does not name one"
    );
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_twice_changes_nothing_and_a_running_caddy_is_given_the_project_without_a_restart()
{
    let rig = rig("launch-twice", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    let blog = rig.project("blog", None);

    rig.start(&shop.id).await.unwrap();
    let caddy_pid = rig.supervisor.status("caddy").unwrap().pid;
    assert_eq!(runs(&rig.reload_runs), 0, "Caddy was started, not reloaded");

    rig.start(&shop.id).await.unwrap();
    assert_eq!(rig.supervisor.status("caddy").unwrap().pid, caddy_pid);

    // A second project: Caddy runs already, so it is reloaded with both sites, and not restarted.
    rig.start(&blog.id).await.unwrap();
    assert_eq!(
        rig.supervisor.status("caddy").unwrap().pid,
        caddy_pid,
        "not restarted"
    );
    assert_eq!(
        runs(&rig.reload_runs),
        2,
        "the second start reloaded (the repeated start reloaded once too)"
    );
    assert_eq!(rig.caddyfile().matches("php_fastcgi").count(), 2);
    assert!(rig.entry(&shop).running && rig.entry(&blog).running);
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_with_no_site_cannot_be_started_and_nothing_is_started_for_it() {
    let rig = rig("launch-nosite", Os::Windows);
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    assert_eq!(rig.entry(&shop).state, SiteState::NoPhp);

    let err = rig.start(&shop.id).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NoSite, "{}", err.message);
    assert_eq!(err.details.unwrap()["reason"], "php");
    assert!(!rig.entry(&shop).served);
    assert!(rig.supervisor.list().is_empty(), "nothing was started");

    // An unknown project, and a folder that has gone.
    assert_eq!(
        rig.start("nope").await.unwrap_err().code,
        ErrorCode::NotFound
    );
    install(&rig.bench.home, "php", "8.3.14");
    fs::remove_dir_all(&shop.path).unwrap();
    let err = rig.start(&shop.id).await.unwrap_err();
    assert_eq!(
        (err.code, err.details.unwrap()["reason"].clone()),
        (ErrorCode::NoSite, "folder".into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_fails_leaves_nothing_it_began_running_and_the_project_not_served() {
    let rig = rig("launch-fail", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", Some(ENV_MYSQL));
    rig.install_database_for(&shop);
    let database = rig.entry(&shop).database_service.unwrap();
    // Somebody holds the port of the site: Caddy cannot start, after the database and the pool are already up.
    let port: u16 = rig.entry(&shop).port.unwrap();
    let holder = TcpListener::bind(("127.0.0.1", port)).unwrap();

    let err = rig.start(&shop.id).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::PortInUse, "{}", err.message);
    assert!(!rig.pool_up(), "the pool this start began is stopped again");
    assert_ne!(
        rig.state(&database),
        Some(ServiceState::Running),
        "so is the database"
    );
    assert_ne!(rig.state("caddy"), Some(ServiceState::Running));
    assert!(!rig.entry(&shop).served, "and the project is not served");
    drop(holder);

    // Once the port is free the same start works.
    rig.start(&shop.id).await.unwrap();
    assert!(rig.entry(&shop).running);
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_start_stops_what_it_began() {
    let rig = rig("launch-cancel", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = rig
        .launcher
        .start(&shop.id, &cancel, &|_| {})
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(rig.supervisor.list().is_empty() || !rig.pool_up());
    assert!(!rig.entry(&shop).served);
}

// ---------------------------------------------------------------------------------------------
// Stop
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_only_project_stops_what_its_start_began() {
    let rig = rig("launch-stop", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", Some(ENV_MYSQL));
    rig.install_database_for(&shop);
    let database = rig.entry(&shop).database_service.unwrap();
    rig.start(&shop.id).await.unwrap();

    rig.launcher.stop(&shop.id).await.unwrap();

    assert!(!rig.pool_up());
    assert_ne!(rig.state(&database), Some(ServiceState::Running));
    assert_ne!(rig.state("caddy"), Some(ServiceState::Running));
    let entry = rig.entry(&shop);
    assert!(!entry.served && !entry.running);
    // Stopping what is not running is a success.
    rig.launcher.stop(&shop.id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_one_of_two_projects_keeps_what_the_other_needs() {
    let rig = rig("launch-shared", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    let blog = rig.project("blog", None);
    rig.start(&shop.id).await.unwrap();
    rig.start(&blog.id).await.unwrap();
    let caddy_pid = rig.supervisor.status("caddy").unwrap().pid;
    let reloads = runs(&rig.reload_runs);

    rig.launcher.stop(&shop.id).await.unwrap();

    // The pool and Caddy are the other project's too: they keep running, and Caddy no longer serves the first.
    assert!(rig.pool_up());
    assert_eq!(
        rig.supervisor.status("caddy").unwrap().pid,
        caddy_pid,
        "not restarted"
    );
    assert_eq!(
        runs(&rig.reload_runs),
        reloads + 1,
        "Caddy took the change by a reload"
    );
    assert_eq!(rig.caddyfile().matches("php_fastcgi").count(), 1);
    assert!(!rig.entry(&shop).running);
    assert!(
        rig.entry(&blog).running,
        "the other project keeps answering"
    );

    // The last one takes everything with it.
    rig.launcher.stop(&blog.id).await.unwrap();
    assert!(!rig.pool_up());
    assert_ne!(rig.state("caddy"), Some(ServiceState::Running));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_user_started_by_hand_is_not_stopped_with_the_project() {
    let rig = rig("launch-by-hand", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    // The user starts the pool and Caddy from the Services page first.
    rig.controller
        .start(&rig.pool(), &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    rig.controller
        .start("caddy", &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    let caddy_pid = rig.supervisor.status("caddy").unwrap().pid;
    assert!(
        !rig.entry(&shop).served,
        "Caddy runs and serves no project yet"
    );

    rig.start(&shop.id).await.unwrap();
    assert_eq!(
        rig.supervisor.status("caddy").unwrap().pid,
        caddy_pid,
        "reloaded, not restarted"
    );
    assert!(rig.entry(&shop).running);

    rig.launcher.stop(&shop.id).await.unwrap();

    assert!(rig.pool_up(), "the pool was not started by the project");
    assert_eq!(
        rig.state("caddy"),
        Some(ServiceState::Running),
        "nor was Caddy"
    );
    assert_eq!(rig.supervisor.status("caddy").unwrap().pid, caddy_pid);
    assert!(
        !rig.caddyfile().contains("php_fastcgi"),
        "but the project is no longer served"
    );
    assert!(!rig.entry(&shop).running);
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_the_user_started_by_hand_or_another_project_needs_is_left_running() {
    let rig = rig("launch-db-shared", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", Some(ENV_MYSQL));
    let blog = rig.project("blog", Some(ENV_MYSQL));
    rig.install_database_for(&shop);
    let database = rig.entry(&shop).database_service.unwrap();

    // The user starts the database from the Services page first: a project does not own it.
    rig.controller
        .start(&database, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    rig.start(&shop.id).await.unwrap();
    rig.launcher.stop(&shop.id).await.unwrap();
    assert_eq!(
        rig.state(&database),
        Some(ServiceState::Running),
        "the database was not started by the project, so the project does not stop it"
    );
    rig.controller.stop(&database).await.unwrap();

    // A database a project started stays while another started project uses the same one.
    rig.start(&shop.id).await.unwrap();
    rig.start(&blog.id).await.unwrap();
    rig.launcher.stop(&shop.id).await.unwrap();
    assert_eq!(
        rig.state(&database),
        Some(ServiceState::Running),
        "the other project still needs it"
    );
    rig.launcher.stop(&blog.id).await.unwrap();
    assert_ne!(
        rig.state(&database),
        Some(ServiceState::Running),
        "the last project that needs it takes it away"
    );
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_services_means_the_project_is_not_running_whatever_was_asked() {
    let rig = rig("launch-derived", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    rig.start(&shop.id).await.unwrap();
    assert!(rig.entry(&shop).running);

    // The user stops Caddy from the Services page: the project is still the one the user started, and it does not answer.
    rig.controller.stop("caddy").await.unwrap();
    let entry = rig.entry(&shop);
    assert!(entry.served && !entry.running, "{entry:?}");
    rig.stop_everything().await;
}

// ---------------------------------------------------------------------------------------------
// As a job
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_start_and_a_stop_are_background_jobs_named_after_the_project() {
    let rig = rig("launch-job", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());

    let started = rig
        .launcher
        .run(&jobs, &shop.id, ProjectAction::Start)
        .unwrap();
    let info = jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Succeeded, "{:?}", info.error);
    assert_eq!(info.key, job_key(&shop.id));
    assert_eq!(info.label, format!("Start project {}", shop.name));
    assert!(rig.entry(&shop).running);

    let stopped = rig
        .launcher
        .run(&jobs, &shop.id, ProjectAction::Stop)
        .unwrap();
    let info = jobs.wait(&stopped.id).await.unwrap();
    assert_eq!(info.state, JobState::Succeeded, "{:?}", info.error);
    assert_eq!(info.label, format!("Stop project {}", shop.name));
    assert!(!rig.entry(&shop).running);

    // A failure is the job's own result, with its code.
    install(&rig.bench.home, "php", "8.3.14");
    fs::remove_dir_all(&shop.path).unwrap();
    let failed = rig
        .launcher
        .run(&jobs, &shop.id, ProjectAction::Start)
        .unwrap();
    let info = jobs.wait(&failed.id).await.unwrap();
    assert_eq!(info.state, JobState::Failed);
    assert_eq!(info.error.unwrap().code, ErrorCode::NoSite);

    assert_eq!(
        rig.launcher
            .run(&jobs, "nope", ProjectAction::Start)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    rig.stop_everything().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caddy_that_rejects_the_project_fails_the_start_and_leaves_the_project_not_served() {
    let rig = rig("launch-invalid", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.project("shop", None);
    let blog = rig.project("blog", None);
    rig.start(&shop.id).await.unwrap();

    // Caddy runs, and now refuses the configuration with the second project in it.
    rig.validate_fails.store(true, Ordering::SeqCst);
    let err = rig.start(&blog.id).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::ConfigInvalid, "{}", err.message);
    assert!(
        !rig.entry(&blog).served,
        "the project that was refused is not served"
    );
    assert!(
        rig.entry(&shop).running,
        "the first project keeps answering"
    );
    rig.validate_fails.store(false, Ordering::SeqCst);
    rig.stop_everything().await;
}

// ---------------------------------------------------------------------------------------------
// The database settings
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_database_settings_are_the_ones_lunarun_runs_and_never_hold_a_password() {
    let rig = rig("launch-dbsettings", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let shop = rig.project("shop", Some(ENV_MYSQL));
    let plain = rig.project("plain", None);

    // The runtime is not installed: nothing to connect to.
    assert_eq!(rig.launcher.database_settings(&shop.id).unwrap(), None);
    assert_eq!(rig.launcher.database_settings(&plain.id).unwrap(), None);

    rig.install_database_for(&shop);
    let settings = rig.launcher.database_settings(&shop.id).unwrap().unwrap();
    assert_eq!(settings.runtime, "mysql");
    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.username, "root");
    assert_eq!(settings.database.as_deref(), Some("shop"));
    assert_eq!(
        settings.port,
        rig.ports
            .assignment(&settings.service)
            .unwrap()
            .unwrap()
            .port,
        "the port the start will use"
    );
    assert_eq!(
        rig.launcher.database_settings(&shop.id).unwrap().unwrap(),
        settings,
        "and it stays"
    );
    assert!(
        !format!("{settings:?}").contains("never-read"),
        "the password of the .env file is never read"
    );
    assert_eq!(rig.launcher.database_settings(&plain.id).unwrap(), None);
    assert_eq!(
        rig.launcher.database_settings("nope").unwrap_err().code,
        ErrorCode::NotFound
    );
}

// ---------------------------------------------------------------------------------------------
// The fixes of the database settings (D-041)
// ---------------------------------------------------------------------------------------------

fn dotenv(project: &Project) -> String {
    fs::read_to_string(Path::new(&project.path).join(".env")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_env_port_is_previewed_and_fixed_only_to_the_port_lunarun_runs() {
    let rig = rig("launch-envport", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let original =
        "DB_CONNECTION=mysql\r\nDB_PORT=1111\r\nDB_DATABASE=shop\r\nDB_PASSWORD=never-read\r\n";
    let shop = rig.project("shop", Some(original));
    let plain = rig.project("plain", None);

    // The database runtime is not installed: lunaRun runs none, so there is no port to compare with.
    assert_eq!(rig.launcher.env_port_preview(&shop.id).unwrap(), None);

    rig.install_database_for(&shop);
    let settings = rig.launcher.database_settings(&shop.id).unwrap().unwrap();
    let preview = rig.launcher.env_port_preview(&shop.id).unwrap().unwrap();
    assert_eq!(preview.port, settings.port);
    assert_eq!(preview.line, 2);
    assert_eq!(preview.before.as_deref(), Some("DB_PORT=1111"));
    assert_eq!(preview.after, format!("DB_PORT={}", settings.port));
    assert_eq!(dotenv(&shop), original, "a preview writes nothing");
    assert!(
        !format!("{preview:?}").contains("never-read"),
        "no other line is in the preview"
    );

    // A preview for another port than the one lunaRun runs is not applied.
    let mut forged = preview.clone();
    forged.port = settings.port.wrapping_add(1);
    forged.after = format!("DB_PORT={}", forged.port);
    assert_eq!(
        rig.launcher
            .env_port_apply(&shop.id, &forged)
            .unwrap_err()
            .code,
        ErrorCode::EnvChanged
    );
    assert_eq!(dotenv(&shop), original);

    let done = rig.launcher.env_port_apply(&shop.id, &preview).unwrap();
    assert_eq!(done.backup, ".env.lunarun-backup");
    assert_eq!(
        fs::read_to_string(Path::new(&shop.path).join(&done.backup)).unwrap(),
        original
    );
    assert_eq!(
        dotenv(&shop),
        original.replace("DB_PORT=1111", &format!("DB_PORT={}", settings.port))
    );
    assert_eq!(rig.launcher.env_port_preview(&shop.id).unwrap(), None);

    // No .env file: nothing to fix, nothing created. An unknown project is not found.
    assert_eq!(rig.launcher.env_port_preview(&plain.id).unwrap(), None);
    assert!(!Path::new(&plain.path).join(".env").exists());
    assert_eq!(
        rig.launcher.env_port_preview("nope").unwrap_err().code,
        ErrorCode::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_database_status_and_create_refuse_what_cannot_be_done_and_say_why() {
    let rig = rig("launch-dbstatus", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let shop = rig.project("shop", Some(ENV_MYSQL));
    let unnamed = rig.project("unnamed", Some("DB_CONNECTION=mysql\n"));
    let odd = rig.project("odd", Some("DB_CONNECTION=mysql\nDB_DATABASE=a;b\n"));
    let plain = rig.project("plain", None);

    // The runtime is not installed.
    let status = |id: &str| rig.launcher.database_status(id).unwrap();
    assert_eq!(
        status(&shop.id),
        luna_core::dbadmin::DatabaseStatus::NotApplicable
    );
    assert_eq!(
        rig.launcher.database_create(&shop.id).unwrap_err().code,
        ErrorCode::InvalidInput
    );

    rig.install_database_for(&shop);
    rig.install_database_for(&unnamed);
    rig.install_database_for(&odd);
    use luna_core::dbadmin::DatabaseStatus as S;
    assert_eq!(status(&unnamed.id), S::NoName);
    assert_eq!(status(&odd.id), S::InvalidName);
    assert_eq!(status(&plain.id), S::NotApplicable);
    // Installed and not started: nothing can be asked of it.
    assert_eq!(status(&shop.id), S::NotRunning);
    let err = rig.launcher.database_create(&shop.id).unwrap_err();
    assert_eq!(err.code, ErrorCode::DatabaseNotRunning);
    assert!(
        err.details.unwrap()["service"]
            .as_str()
            .unwrap()
            .starts_with("mysql@")
    );
    // No name, or one that is not plain: refused before anything is asked of a server.
    assert_eq!(
        rig.launcher.database_create(&unnamed.id).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        rig.launcher.database_create("nope").unwrap_err().code,
        ErrorCode::NotFound
    );
}
