//! Opt-in check of the sites of the projects against the **real PHP and Caddy** of this platform (FR-ST-01 to FR-ST-03;
//! D-038). The newest PHP and Caddy with a build for this computer are installed by the real installer, two projects are
//! added to the real project store, the real `ServiceController` starts the PHP pool and Caddy **with the sites the real
//! `SiteManager` planned**, and pages are requested through Caddy. Then a project is added while Caddy runs and the
//! sites are synced: Caddy validates and reloads (no restart: the first site keeps answering and Caddy keeps its
//! process), and a removed project stops being served.
//!
//! It downloads about 40 MB, runs third-party programs from the internet and takes a minute or two, so it is ignored by
//! default:
//!
//!     cargo test -p luna-core --test sites_online -- --ignored --nocapture
//!
//! It proves only the platform it runs on. The home and the project folders have a space and a non-ASCII letter in their
//! paths (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::projects::{DetectionData, ProjectStore, Surroundings};
use luna_core::registry::{Registry, RegistryHolder};
use luna_core::runtimes::{Installer, list_installed};
use luna_core::services::{PortAllocator, ServiceController, ServiceState, Supervisor};
use luna_core::sites::{SiteManager, SiteState, SyncOutcome, WebServer};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

/// Install the newest release of `runtime` that has a build for this computer, through the real installer.
async fn install(home: &LunaHome, jobs: &JobManager, runtime: &str) -> String {
    let (os, arch) = (Os::current(), Arch::current());
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
    let installer = Installer::new(
        home.clone(),
        Downloader::new(DownloadOptions::default()).unwrap(),
    );
    let started = installer
        .start(jobs, Arc::new(registry), runtime, &version)
        .unwrap();
    let job = jobs.wait(&started.id).await.unwrap();
    assert_eq!(
        job.state,
        JobState::Succeeded,
        "{runtime} install failed: {:?}",
        job.error
    );
    version
}

/// One plain HTTP/1.0 request to `127.0.0.1:<port>`. Returns the status and the body.
async fn get(port: u16, path: &str) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
        .await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map_or(String::new(), |(_, body)| body.to_owned());
    Ok((status, body))
}

/// Ask until the answer is 200, for up to `limit`: the first request can arrive while a worker is warming up.
async fn get_ok(port: u16, path: &str, limit: Duration) -> String {
    let until = Instant::now() + limit;
    loop {
        let answer = get(port, path).await;
        if let Ok((200, body)) = &answer {
            return body.clone();
        }
        assert!(
            Instant::now() < until,
            "GET 127.0.0.1:{port}{path} never answered 200: {answer:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

const PAGE: &str = "<?php echo 'hello from ', basename(dirname(__DIR__)), ' on php ', PHP_VERSION;";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 40 MB and runs a real PHP and Caddy"]
async fn real_caddy_serves_the_sites_of_the_projects_and_takes_a_change_without_a_restart() {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let os = Os::current();

    // 1. The real runtimes, through the real installer.
    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
    let php_version = install(&home, &jobs, "php").await;
    install(&home, &jobs, "caddy").await;

    // 2. Two projects in folders with a space and a non-ASCII letter, added to the real store.
    let projects = tmp.path().join("Projekte größe");
    let make = |name: &str| {
        let dir = projects.join(name);
        fs::create_dir_all(dir.join("public")).unwrap();
        fs::write(dir.join("public").join("index.php"), PAGE).unwrap();
        fs::write(dir.join("public").join("hello.txt"), "static ok").unwrap();
        fs::write(dir.join("composer.json"), r#"{"require":{"php":">=8.1"}}"#).unwrap();
        dir
    };
    let (shop_dir, blog_dir) = (make("shop"), make("blog"));
    let data = Arc::new(DetectionData::bundled().unwrap());
    let store = ProjectStore::open(&home).unwrap();
    let around = Surroundings {
        installed: &[],
        available: &[],
        os,
    };
    let shop = store.add(&shop_dir, &data, &around).unwrap().project;

    // 3. The real site manager and the real controller, which asks it for the sites each time it plans Caddy.
    let supervisor = Supervisor::new(Handle::current(), home.clone(), |_| {});
    let ports = PortAllocator::new(home.clone());
    let sites = SiteManager::new(
        home.clone(),
        store.clone(),
        Arc::clone(&data),
        RegistryHolder::with_bundled().unwrap(),
        ports.clone(),
        supervisor.clone(),
        os,
    );
    let source = sites.clone();
    let controller = ServiceController::new(home.clone(), ports.clone(), supervisor.clone(), os)
        .with_sites(move || source.sites());

    let planned = sites.plan().unwrap();
    assert_eq!(planned.entries[0].state, SiteState::Ready, "{planned:?}");
    assert_eq!(
        planned.entries[0].php_version.as_deref(),
        Some(php_version.as_str())
    );
    let shop_port = planned.entries[0].port.unwrap();
    eprintln!("shop is http://127.0.0.1:{shop_port}, PHP {php_version}");

    // 4. Start the pool and Caddy as the Services page does; Caddy serves the project that was added before it started.
    let pool = format!(
        "{}@{php_version}",
        if os == Os::Windows {
            "php-fcgi"
        } else {
            "php-fpm"
        }
    );
    let cancel = CancellationToken::new();
    controller.start(&pool, &cancel, &|_| {}).await.unwrap();
    controller.start("caddy", &cancel, &|_| {}).await.unwrap();
    let body = get_ok(shop_port, "/", Duration::from_secs(30)).await;
    assert!(body.contains("hello from shop"), "{body}");
    assert!(body.contains(&format!("php {php_version}")), "{body}");
    assert_eq!(
        get_ok(shop_port, "/hello.txt", Duration::from_secs(5)).await,
        "static ok"
    );
    assert_eq!(sites.view().unwrap().web_server, WebServer::Running);

    // 5. A project is added while Caddy runs: validated and reloaded, without a restart.
    let caddy_pid = supervisor.status("caddy").unwrap().pid;
    let blog = store.add(&blog_dir, &data, &around).unwrap().project;
    assert_eq!(sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    let blog_port = sites
        .plan()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.project_id == blog.id)
        .and_then(|entry| entry.port)
        .unwrap();
    assert_ne!(blog_port, shop_port);
    let body = get_ok(blog_port, "/", Duration::from_secs(30)).await;
    assert!(body.contains("hello from blog"), "{body}");
    get_ok(shop_port, "/", Duration::from_secs(5)).await;
    assert_eq!(
        supervisor.status("caddy").unwrap().pid,
        caddy_pid,
        "Caddy was not restarted"
    );
    assert_eq!(
        supervisor.status("caddy").unwrap().state,
        ServiceState::Running
    );
    assert_eq!(sites.sync().await.unwrap(), SyncOutcome::Unchanged);

    // 6. A project is removed: it stops being served, the other keeps answering.
    store.remove(&shop.id).unwrap();
    assert_eq!(sites.sync().await.unwrap(), SyncOutcome::Reloaded);
    let until = Instant::now() + Duration::from_secs(10);
    while get(shop_port, "/")
        .await
        .is_ok_and(|(status, _)| status == 200)
    {
        assert!(
            Instant::now() < until,
            "the removed project is still served"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    get_ok(blog_port, "/", Duration::from_secs(5)).await;
    assert_eq!(supervisor.status("caddy").unwrap().pid, caddy_pid);
    assert!(
        ports
            .assignment(&format!("site@{}", shop.id))
            .unwrap()
            .is_none(),
        "its port is free again"
    );

    // 7. Stop in the order of a quit: Caddy first, then PHP.
    controller.stop("caddy").await.unwrap();
    controller.stop(&pool).await.unwrap();
    assert_eq!(
        list_installed(&home).unwrap().len(),
        2,
        "nothing but the two runtimes is installed"
    );
}
