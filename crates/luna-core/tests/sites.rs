//! The sites of the projects (FR-ST-01 to FR-ST-03; D-038): which projects get a site and on which port, that the port
//! stays, what is written into the Caddyfile, that a running Caddy is told only when something changed and only after
//! the file passed validation, and that a removed project gives its port back. The runtimes are folders with receipts and
//! `caddy validate` and `caddy reload` are replaced by the stand-in program; the real Caddy is run by the ignored
//! `web_online` test and the real app window. Every data folder and every project folder has a space and a
//! non-ASCII letter in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use luna_core::diagnostics::{Arch, Os};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::projects::{
    Binding, BindingChoice, BindingMode, DetectionData, DocRootChoice, Project, ProjectPatch,
    ProjectStore, Surroundings,
};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::InstallReceipt;
use luna_core::services::{
    PortAllocator, PortPurpose, PortRanges, ServiceController, ServiceState, StartPlan, Supervisor,
    pool_ports,
};
use luna_core::sites::{SiteManager, SiteState, SyncOutcome, WebServer};
use luna_core::{ErrorCode, LunaHome};
use stub_service::*;
use tokio::runtime::Handle;

struct Rig {
    bench: Shared,
    store: ProjectStore,
    ports: PortAllocator,
    supervisor: Supervisor,
    sites: SiteManager,
    os: Os,
    /// While set, the stand-in `caddy validate` / `caddy reload` fail.
    validate_fails: Arc<AtomicBool>,
    reload_fails: Arc<AtomicBool>,
    validate_runs: PathBuf,
    reload_runs: PathBuf,
}

fn rig(test: &str, os: Os) -> Rig {
    let bench = private(test);
    let (supervisor, _events) = bench.supervisor();
    let ports = PortAllocator::new(bench.home.clone()).with_ranges(PortRanges {
        site: free_block(24),
        php_fastcgi: free_block(16),
        caddy_admin: free_block(1),
        ..PortRanges::default()
    });
    let store = ProjectStore::open(&bench.home).unwrap();
    let scratch = bench.scratch(test);
    let validate_fails = Arc::new(AtomicBool::new(false));
    let reload_fails = Arc::new(AtomicBool::new(false));
    let (validate_runs, reload_runs) = (scratch.join("validate-runs"), scratch.join("reload-runs"));
    let for_hook = Shared {
        home: bench.home.clone(),
        stub: bench.stub.clone(),
    };
    let (vf, rf) = (Arc::clone(&validate_fails), Arc::clone(&reload_fails));
    let (vr, rr) = (validate_runs.clone(), reload_runs.clone());
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
        let data = data_dir.to_str().unwrap();
        let ok = |counter: &Path| {
            for_hook.helper(
                "init-ok",
                &[
                    ("LUNA_STUB_DATADIR", data),
                    ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
                ],
            )
        };
        let bad = || for_hook.helper("exit", &[("LUNA_STUB_EXIT_CODE", "1")]);
        definition.validate_command = if vf.load(Ordering::SeqCst) {
            bad()
        } else {
            ok(&vr)
        };
        definition.reload_command = if rf.load(Ordering::SeqCst) {
            bad()
        } else {
            ok(&rr)
        };
    });
    Rig {
        bench,
        store,
        ports,
        supervisor,
        sites,
        os,
        validate_fails,
        reload_fails,
        validate_runs,
        reload_runs,
    }
}

/// An installed runtime: a folder with a receipt, as the installer leaves it.
fn install(home: &LunaHome, runtime: &str, version: &str) {
    let dir = home.runtime_dir(runtime, version).unwrap();
    fs::create_dir_all(dir.join("bin")).unwrap();
    InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
        .write(&dir)
        .unwrap();
}

impl Rig {
    /// A project folder (space and non-ASCII letter in its name) with `files`, added to the store.
    fn project(&self, name: &str, files: &[(&str, &str)]) -> Project {
        let dir = self.bench.scratch("projects").join(format!("mon {name} é"));
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let around = Surroundings {
            installed: &[],
            available: &[],
            os: self.os,
        };
        self.store
            .add(&dir, &DetectionData::bundled().unwrap(), &around)
            .unwrap()
            .project
    }

    fn laravel(&self, name: &str) -> Project {
        self.project(
            name,
            &[
                ("artisan", ""),
                ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
                ("public/index.php", "<?php"),
            ],
        )
    }

    fn caddyfile(&self) -> String {
        fs::read_to_string(self.bench.home.config_dir().join("caddy").join("Caddyfile")).unwrap()
    }

    fn update(&self, id: &str, patch: &ProjectPatch) {
        self.store
            .update(id, patch, &DetectionData::bundled().unwrap())
            .unwrap();
    }

    /// Start a stand-in "caddy" so the supervisor reports it running.
    async fn run_caddy(&self) {
        let admin = self
            .ports
            .assign("caddy", PortPurpose::CaddyAdmin)
            .unwrap()
            .port;
        self.supervisor
            .start(self.bench.serving("caddy", admin, &[]))
            .unwrap();
        self.supervisor
            .wait_until_settled("caddy", Duration::from_secs(20))
            .await
            .unwrap();
    }

    async fn stop_caddy(&self) {
        let _ = self.supervisor.stop("caddy").await;
    }
}

fn forward(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

// ---------------------------------------------------------------------------------------------
// Who gets a site
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_project_with_an_installed_php_gets_a_site_on_a_port_that_stays() {
    let rig = rig("sites-port", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let shop = rig.laravel("shop");
    let blog = rig.laravel("blog");

    let plan = rig.sites.plan().unwrap();
    assert_eq!(plan.entries.len(), 2);
    let (a, b) = (&plan.entries[0], &plan.entries[1]);
    assert_eq!(
        (a.project_id.as_str(), a.state),
        (shop.id.as_str(), SiteState::Ready)
    );
    assert_eq!(
        (b.project_id.as_str(), b.state),
        (blog.id.as_str(), SiteState::Ready)
    );
    assert_eq!(a.php_version.as_deref(), Some("8.3.14"));
    let (pa, pb) = (a.port.unwrap(), b.port.unwrap());
    assert_ne!(pa, pb, "one port each");
    assert_eq!(a.address, Some(format!("http://127.0.0.1:{pa}")));
    assert_eq!(plan.sites.len(), 2);

    // The same ports next time, and after a restart of lunaRun (a new manager over the same files).
    let again = rig.sites.plan().unwrap();
    assert_eq!(again.entries, plan.entries);
    let restarted = ProjectStore::open(&rig.bench.home).unwrap();
    assert_eq!(restarted.list().len(), 2);
    let ports_after = PortAllocator::new(rig.bench.home.clone());
    assert_eq!(
        ports_after
            .assignment(&format!("site@{}", shop.id))
            .unwrap()
            .unwrap()
            .port,
        pa
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_is_the_projects_document_root_served_by_the_pool_of_its_php() {
    for os in [Os::Windows, Os::Linux] {
        let rig = rig(&format!("sites-site-{os:?}"), os);
        install(&rig.bench.home, "php", "8.3.14");
        let shop = rig.laravel("shop");
        let plain = rig.project(
            "plain",
            &[
                ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
                ("index.php", "<?php"),
            ],
        );

        let sites = rig.sites.plan().unwrap().sites;
        assert_eq!(
            sites[0].root,
            Path::new(&shop.path).join("public"),
            "Laravel serves public/"
        );
        // As text, because `Path` equality ignores a trailing `.` and the Caddyfile does not.
        assert_eq!(
            sites[1].root.to_str().unwrap(),
            plain.path,
            "the folder itself, not `folder/.`"
        );
        let pool = pool_ports(&rig.ports, os, "8.3.14").unwrap();
        assert_eq!(sites[0].php_ports, pool, "{os:?}");
        assert_eq!(pool.len(), if os == Os::Windows { 4 } else { 1 });
        assert!(
            pool.windows(2).all(|w| w[1] == w[0] + 1),
            "a Windows pool is consecutive ports"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_whose_php_is_not_installed_has_no_site_until_it_is() {
    let rig = rig("sites-nophp", Os::Windows);
    let shop = rig.laravel("shop");

    let before = rig.sites.plan().unwrap();
    assert_eq!(before.entries[0].state, SiteState::NoPhp);
    assert_eq!(
        (before.entries[0].port, before.entries[0].address.clone()),
        (None, None)
    );
    assert!(before.sites.is_empty());
    assert_eq!(
        rig.ports.assignment(&format!("site@{}", shop.id)).unwrap(),
        None,
        "no port is kept for a site that does not exist"
    );

    install(&rig.bench.home, "php", "8.3.14");
    let after = rig.sites.plan().unwrap();
    assert_eq!(after.entries[0].state, SiteState::Ready);
    assert!(after.entries[0].port.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn php_the_user_bound_by_hand_decides_which_php_the_site_needs() {
    let rig = rig("sites-bound", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let shop = rig.laravel("shop");
    // 8.1 is not installed: the project is bound to it, so there is no site, whatever else is installed.
    rig.update(
        &shop.id,
        &ProjectPatch {
            php: Some(BindingChoice::Use {
                binding: Binding {
                    mode: BindingMode::Line,
                    value: "8.1".to_owned(),
                },
            }),
            ..ProjectPatch::default()
        },
    );
    assert_eq!(rig.sites.plan().unwrap().entries[0].state, SiteState::NoPhp);

    install(&rig.bench.home, "php", "8.1.30");
    let plan = rig.sites.plan().unwrap();
    assert_eq!(plan.entries[0].php_version.as_deref(), Some("8.1.30"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_has_gone_has_no_site_and_keeps_no_port() {
    let rig = rig("sites-gone", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let shop = rig.laravel("shop");
    fs::remove_dir_all(&shop.path).unwrap();

    let plan = rig.sites.plan().unwrap();
    assert_eq!(plan.entries[0].state, SiteState::FolderMissing);
    assert!(plan.sites.is_empty());
    assert_eq!(
        rig.ports.assignment(&format!("site@{}", shop.id)).unwrap(),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_document_root_the_user_chose_wins_and_an_automatic_one_follows_the_folder() {
    let rig = rig("sites-docroot", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let project = rig.project(
        "root",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
            ("web/index.php", "<?php"),
        ],
    );
    assert_eq!(
        rig.sites.plan().unwrap().sites[0].root,
        PathBuf::from(&project.path)
    );

    // The project gains public/index.php: an automatic document root follows it without a rescan.
    fs::create_dir_all(Path::new(&project.path).join("public")).unwrap();
    fs::write(Path::new(&project.path).join("public/index.php"), "<?php").unwrap();
    assert_eq!(
        rig.sites.plan().unwrap().sites[0].root,
        Path::new(&project.path).join("public")
    );

    rig.update(
        &project.id,
        &ProjectPatch {
            doc_root: Some(DocRootChoice::Use {
                path: "web".to_owned(),
            }),
            ..ProjectPatch::default()
        },
    );
    assert_eq!(
        rig.sites.plan().unwrap().sites[0].root,
        Path::new(&project.path).join("web")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn site_names_are_safe_unique_and_short_whatever_the_project_is_called() {
    let rig = rig("sites-names", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    for (n, name) in [
        "Café ☕ shop!!",
        "###",
        "a very long project name that goes on and on and on",
    ]
    .into_iter()
    .enumerate()
    {
        let project = rig.laravel(&format!("n{n}"));
        rig.store
            .update(
                &project.id,
                &ProjectPatch {
                    name: Some(name.to_owned()),
                    ..ProjectPatch::default()
                },
                &DetectionData::bundled().unwrap(),
            )
            .unwrap();
    }
    let twin_a = rig.laravel("twin1");
    let twin_b = rig.laravel("twin2");
    for twin in [&twin_a, &twin_b] {
        rig.store
            .update(
                &twin.id,
                &ProjectPatch {
                    name: Some("Same".to_owned()),
                    ..ProjectPatch::default()
                },
                &DetectionData::bundled().unwrap(),
            )
            .unwrap();
    }

    let plan = rig.sites.plan().unwrap();
    let names: Vec<&str> = plan.entries.iter().map(|e| e.name.as_str()).collect();
    for name in &names {
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && name.len() <= 40,
            "{name}"
        );
    }
    assert!(names[0].starts_with("caf-shop-"), "{}", names[0]);
    assert!(names[1].starts_with("site-"), "{}", names[1]);
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        names.len(),
        "two projects called Same still differ: {names:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// What Caddy is told
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_caddyfile_holds_a_block_per_site_and_nothing_is_written_without_caddy() {
    let rig = rig("sites-caddyfile", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    let shop = rig.laravel("shop");
    rig.laravel("blog");

    // No Caddy installed: sites are planned, nothing is written.
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::NoWebServer);
    assert!(
        !rig.bench
            .home
            .config_dir()
            .join("caddy")
            .join("Caddyfile")
            .exists()
    );

    install(&rig.bench.home, "caddy", "2.11.7");
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Written);
    let text = rig.caddyfile();
    let plan = rig.sites.plan().unwrap();
    for site in &plan.sites {
        let port = site.port;
        assert!(
            text.contains(&format!(
                "http://localhost:{port}, http://127.0.0.1:{port} {{"
            )),
            "{text}"
        );
        let upstreams: Vec<String> = site
            .php_ports
            .iter()
            .map(|p| format!("127.0.0.1:{p}"))
            .collect();
        assert!(
            text.contains(&format!("php_fastcgi {}", upstreams.join(" "))),
            "{text}"
        );
        assert!(
            text.contains(&format!("root * \"{}\"", forward(&site.root))),
            "{text}"
        );
    }
    assert!(text.contains(&forward(&Path::new(&shop.path).join("public"))));
    assert_eq!(text.matches("php_fastcgi").count(), 2);
    // Nothing was run: Caddy is not running.
    assert_eq!(launches(&rig.validate_runs), 0);
    assert_eq!(launches(&rig.reload_runs), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_caddy_is_validated_then_reloaded_and_only_when_something_changed() {
    let rig = rig("sites-reload", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    rig.laravel("shop");
    rig.run_caddy().await;

    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    assert_eq!(
        (launches(&rig.validate_runs), launches(&rig.reload_runs)),
        (1, 1)
    );

    // Nothing changed: Caddy is left alone (no pointless reload).
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Unchanged);
    assert_eq!(
        (launches(&rig.validate_runs), launches(&rig.reload_runs)),
        (1, 1)
    );

    // A project is added: the new site is given to the running Caddy, with no restart.
    let pid_before = rig.supervisor.status("caddy").unwrap().pid;
    let blog = rig.laravel("blog");
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    assert_eq!(
        (launches(&rig.validate_runs), launches(&rig.reload_runs)),
        (2, 2)
    );
    assert!(
        rig.caddyfile()
            .contains(&forward(&Path::new(&blog.path).join("public")))
    );
    assert_eq!(
        rig.supervisor.status("caddy").unwrap().pid,
        pid_before,
        "not restarted"
    );

    // A project is removed: it stops being served.
    rig.store.remove(&blog.id).unwrap();
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    assert!(!rig.caddyfile().contains(&forward(Path::new(&blog.path))));
    rig.stop_caddy().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caddyfile_caddy_rejects_never_reaches_it_and_the_next_try_goes_through() {
    let rig = rig("sites-invalid", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    rig.laravel("shop");
    rig.run_caddy().await;
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    rig.laravel("blog");

    rig.validate_fails.store(true, Ordering::SeqCst);
    let err = rig.sites.sync().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid, "{}", err.message);
    assert_eq!(
        launches(&rig.reload_runs),
        1,
        "no reload after a failed validation"
    );
    let view = rig.sites.view().unwrap();
    assert_eq!(
        view.problem.as_ref().map(|p| p.code),
        Some(ErrorCode::ConfigInvalid)
    );
    assert_eq!(view.web_server, WebServer::Running);
    assert_eq!(
        rig.supervisor.status("caddy").unwrap().state,
        ServiceState::Running,
        "Caddy keeps running"
    );

    // The cause is gone: the same Caddyfile, unchanged on disk, is tried again (a failure must not be remembered as done).
    rig.validate_fails.store(false, Ordering::SeqCst);
    assert_eq!(rig.sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    assert_eq!(launches(&rig.reload_runs), 2);
    assert!(
        rig.sites.view().unwrap().problem.is_none(),
        "the problem went away with it"
    );

    // A reload that fails is a problem of its own.
    rig.laravel("third");
    rig.reload_fails.store(true, Ordering::SeqCst);
    let err = rig.sites.sync().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ReloadFailed, "{}", err.message);
    assert_eq!(
        rig.sites.view().unwrap().problem.map(|p| p.code),
        Some(ErrorCode::ReloadFailed)
    );
    rig.stop_caddy().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_project_gives_its_port_back() {
    let rig = rig("sites-release", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.laravel("shop");
    let port = rig.sites.plan().unwrap().entries[0].port.unwrap();
    assert!(
        rig.ports
            .assignment(&format!("site@{}", shop.id))
            .unwrap()
            .is_some()
    );

    rig.store.remove(&shop.id).unwrap();
    rig.sites.sync().await.unwrap();

    assert_eq!(
        rig.ports.assignment(&format!("site@{}", shop.id)).unwrap(),
        None
    );
    // The next project can have it.
    rig.laravel("next");
    assert_eq!(rig.sites.plan().unwrap().entries[0].port, Some(port));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_view_says_whether_there_is_a_web_server_to_serve_the_sites() {
    let rig = rig("sites-view", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    rig.laravel("shop");
    assert_eq!(
        rig.sites.view().unwrap().web_server,
        WebServer::NotInstalled
    );

    install(&rig.bench.home, "caddy", "2.11.7");
    assert_eq!(rig.sites.view().unwrap().web_server, WebServer::Stopped);

    rig.run_caddy().await;
    assert_eq!(rig.sites.view().unwrap().web_server, WebServer::Running);
    rig.stop_caddy().await;
}

// ---------------------------------------------------------------------------------------------
// The Services page's Caddy
// ---------------------------------------------------------------------------------------------

fn controller(rig: &Rig) -> ServiceController {
    let sites = rig.sites.clone();
    let for_hook = Shared {
        home: rig.bench.home.clone(),
        stub: rig.bench.stub.clone(),
    };
    ServiceController::new(
        rig.bench.home.clone(),
        rig.ports.clone(),
        rig.supervisor.clone(),
        rig.os,
    )
    .with_sites(move || sites.sites())
    .with_plan_hook(move |plan| {
        if let StartPlan::Web(def) = plan {
            // The stand-in listens on the admin port; the ports to check before the start stay the real ones.
            let checked = def.spec.ports.clone();
            def.spec = for_hook.serving("caddy", def.admin_port, &[]);
            def.spec.ports = checked;
            def.validate_command = for_hook.helper("noop", &[]);
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn caddy_started_from_the_services_page_serves_the_sites_of_the_projects() {
    let rig = rig("sites-controller", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    rig.laravel("shop");
    rig.laravel("blog");
    let controller = controller(&rig);

    let StartPlan::Web(def) = controller.plan("caddy").unwrap() else {
        panic!("the caddy row plans Caddy");
    };
    assert_eq!(def.sites.len(), 2);
    for site in &def.sites {
        assert!(
            def.spec.ports.contains(&site.port),
            "its ports are checked before the start"
        );
    }
    assert!(
        rig.caddyfile().contains("php_fastcgi"),
        "the Caddyfile of the start has the sites"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn use_another_port_moves_the_site_whose_port_is_taken() {
    let rig = rig("sites-port-taken", Os::Windows);
    install(&rig.bench.home, "php", "8.3.14");
    install(&rig.bench.home, "caddy", "2.11.7");
    let shop = rig.laravel("shop");
    let blog = rig.laravel("blog");
    let controller = controller(&rig);
    let plan = rig.sites.plan().unwrap();
    let (shop_port, blog_port) = (plan.entries[0].port.unwrap(), plan.entries[1].port.unwrap());
    let admin = rig
        .ports
        .assign("caddy", PortPurpose::CaddyAdmin)
        .unwrap()
        .port;
    let holder = TcpListener::bind(("127.0.0.1", shop_port)).unwrap();

    let err = controller
        .start(
            "caddy",
            &tokio_util::sync::CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse, "{}", err.message);
    assert_eq!(err.details.as_ref().unwrap()["port"], shop_port);

    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
    let started = controller.reassign_and_start(&jobs, "caddy").unwrap();
    let info = jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Succeeded, "{:?}", info.error);
    drop(holder);

    let after = rig.sites.plan().unwrap();
    assert_ne!(
        after.entries[0].port.unwrap(),
        shop_port,
        "the taken site moved"
    );
    assert_eq!(
        after.entries[1].port.unwrap(),
        blog_port,
        "the other site did not"
    );
    assert_eq!(
        rig.ports.assignment("caddy").unwrap().unwrap().port,
        admin,
        "nor did the admin port"
    );
    assert!(
        rig.caddyfile()
            .contains(&format!("localhost:{}", after.entries[0].port.unwrap()))
    );
    assert_eq!(
        rig.supervisor.status("caddy").unwrap().state,
        ServiceState::Running
    );
    let _ = (shop, blog);
    controller.stop("caddy").await.unwrap();
}
