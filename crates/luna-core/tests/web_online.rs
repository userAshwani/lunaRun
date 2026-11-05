//! Opt-in check of the PHP and Caddy services against the **real PHP and Caddy** of this platform. The newest PHP and
//! Caddy with a build for this computer are downloaded by the real installer (checked against the registry's SHA-256),
//! a PHP pool and Caddy are started through the supervisor, a PHP page is requested **through Caddy**, a second site
//! is added with a reload (no restart, the first keeps answering), and everything is stopped in the order of the app's
//! Quit: Caddy first, then PHP (SERVICES section 2).
//!
//! It downloads about 40 MB, runs third-party programs from the internet and takes a minute or two, so it is ignored
//! by default:
//!
//!     cargo test -p luna-core --test web_online -- --ignored --nocapture
//!
//! It proves only the platform it runs on. The home and the project folder have a space and a non-ASCII letter in
//! their paths, which is the real test of the generated `php.ini`, `Caddyfile` and arguments (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::registry::Registry;
use luna_core::runtimes::{InstalledRuntime, Installer, list_installed};
use luna_core::services::{
    PortAllocator, PortPurpose, ServiceState, ServiceStatus, Site, Supervisor, caddy_definition,
    php_definition, port_owners,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

/// Install the newest release of `runtime` that has a build for this computer, through the real installer.
async fn install(home: &LunaHome, jobs: &JobManager, runtime: &str) -> InstalledRuntime {
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
    list_installed(home)
        .unwrap()
        .into_iter()
        .find(|i| i.runtime == runtime)
        .unwrap()
}

/// One plain HTTP/1.0 request to `127.0.0.1:<port>` with the `Host` a browser would send. Returns the status and the
/// body. HTTP/1.0 so the answer is not chunked and ends when the connection does.
async fn get(port: u16, host: &str, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map_or(String::new(), |(_, body)| body.to_owned());
    (status, body)
}

/// Ask until the answer is 200, for up to `limit`: the first request can arrive while a worker is still warming up.
async fn get_ok(port: u16, host: &str, path: &str, limit: Duration) -> String {
    let until = Instant::now() + limit;
    loop {
        let (status, body) = get(port, host, path).await;
        if status == 200 || Instant::now() >= until {
            assert_eq!(status, 200, "GET {host}{path}: {body}");
            return body;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn tail(path: &Path) -> String {
    // Bytes, not `read_to_string`: a program may write a path in the ANSI code page.
    fs::read(path).map_or_else(
        |_| "(no log)".to_owned(),
        |bytes| {
            let text = String::from_utf8_lossy(&bytes);
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(25)..].join("\n")
        },
    )
}

async fn running(supervisor: &Supervisor, home: &LunaHome, id: &str) -> ServiceStatus {
    let status = supervisor
        .wait_until_settled(id, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(
        status.state,
        ServiceState::Running,
        "{id} did not start: {:?}\n--- its output:\n{}",
        status.error,
        tail(&home.logs_dir().join(format!("{id}.log")))
    );
    status
}

/// A normalised form for comparing paths PHP prints: forward slashes, lower case.
fn norm(path: impl AsRef<Path>) -> String {
    path.as_ref()
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

const FRONT_CONTROLLER: &str = r#"<?php
echo "hello from php ", PHP_VERSION, "\n";
echo "ini: ", php_ini_loaded_file(), "\n";
echo "scanned: ", php_ini_scanned_files() ?: "none", "\n";
echo "uri: ", $_SERVER['REQUEST_URI'], "\n";
echo "script: ", $_SERVER['SCRIPT_FILENAME'], "\n";
"#;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 40 MB and runs a real PHP and Caddy"]
async fn real_php_and_caddy_serve_a_php_page_reload_a_new_site_and_stop_cleanly() {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let os = Os::current();

    // 1. The real runtimes, through the real installer.
    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
    let php_runtime = install(&home, &jobs, "php").await;
    let caddy_runtime = install(&home, &jobs, "caddy").await;

    // 2. Two projects in folders with a space and a non-ASCII letter: the user's own folders, outside LUNA_HOME.
    let projects = tmp.path().join("Projekte größe");
    let shop = projects.join("shop").join("public");
    let blog = projects.join("blog").join("public");
    for root in [&shop, &blog] {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("index.php"), FRONT_CONTROLLER).unwrap();
        fs::write(root.join("hello.txt"), "static ok").unwrap();
    }

    // 3. A PHP pool and Caddy with the first site, started through the supervisor.
    let ports = PortAllocator::new(home.clone());
    let php = php_definition(&home, &ports, &php_runtime, os, 2).unwrap();
    let shop_port = ports.assign("site@shop", PortPurpose::Site).unwrap().port;
    let one_site = [Site {
        name: "shop".to_owned(),
        port: shop_port,
        root: shop.clone(),
        php_ports: php.ports.clone(),
    }];
    let caddy = caddy_definition(&home, &ports, &caddy_runtime, &one_site, os).unwrap();
    eprintln!(
        "php pool {} on {:?}, caddy admin {} and site {shop_port}",
        php.pool_id, php.ports, caddy.admin_port
    );
    eprintln!(
        "--- Caddyfile:\n{}",
        fs::read_to_string(&caddy.config_file).unwrap()
    );
    caddy
        .validate(Duration::from_secs(60), &CancellationToken::new())
        .await
        .unwrap_or_else(|error| {
            panic!("caddy validate failed: {error}\n{}", tail(&caddy.check_log))
        });

    let supervisor = Supervisor::new(Handle::current(), home.clone(), |_: ServiceStatus| {})
        .with_poll_interval(Duration::from_millis(250));
    for spec in &php.specs {
        spec.validate(&home).unwrap();
        supervisor.start(spec.clone()).unwrap();
    }
    for spec in &php.specs {
        let status = running(&supervisor, &home, &spec.id).await;
        eprintln!("{} running (pid {:?})", spec.id, status.pid);
    }
    caddy.spec.validate(&home).unwrap();
    supervisor.start(caddy.spec.clone()).unwrap();
    let caddy_status = running(&supervisor, &home, "caddy").await;
    eprintln!("caddy running (pid {:?})", caddy_status.pid);
    assert!(
        !port_owners(shop_port).is_empty(),
        "the OS says Caddy listens"
    );

    // 4. A PHP page through Caddy: PHP found the script in a folder with a space and an `ü`, read the generated
    //    php.ini and the user's conf.d, and Caddy sent a route that is not a file to the front controller.
    let host = format!("localhost:{shop_port}");
    let page = get_ok(shop_port, &host, "/", Duration::from_secs(30)).await;
    eprintln!("--- GET /:\n{page}");
    assert!(
        page.contains(&format!("hello from php {}", php_runtime.version)),
        "{page}"
    );
    let ini = luna_core::runtimes::php_config_paths(&home, &php_runtime.version)
        .unwrap()
        .ini;
    assert!(
        page.lines()
            .any(|l| l.starts_with("ini: ") && norm(&l[5..]) == norm(&ini)),
        "PHP read the generated php.ini {}:\n{page}",
        ini.display()
    );
    assert!(
        page.contains("99-user.ini"),
        "PHP scanned the user's conf.d:\n{page}"
    );
    let routed = get_ok(shop_port, &host, "/orders/42", Duration::from_secs(10)).await;
    assert!(routed.contains("uri: /orders/42"), "{routed}");
    assert!(
        routed
            .lines()
            .any(|l| l.starts_with("script: ") && norm(&l[8..]) == norm(shop.join("index.php"))),
        "the front controller of the site ran:\n{routed}"
    );
    // The same site by its other name, and a file that exists is served as a file.
    let by_ip = get_ok(
        shop_port,
        &format!("127.0.0.1:{shop_port}"),
        "/",
        Duration::from_secs(10),
    )
    .await;
    assert!(by_ip.contains("hello from php"));
    assert_eq!(
        get_ok(shop_port, &host, "/hello.txt", Duration::from_secs(10)).await,
        "static ok"
    );

    // 5. Add a second site and reload: no restart (same process), the first site keeps answering.
    let blog_port = ports.assign("site@blog", PortPurpose::Site).unwrap().port;
    let two_sites = [
        one_site[0].clone(),
        Site {
            name: "blog".to_owned(),
            port: blog_port,
            root: blog.clone(),
            php_ports: php.ports.clone(),
        },
    ];
    let reloaded = caddy_definition(&home, &ports, &caddy_runtime, &two_sites, os).unwrap();
    reloaded
        .validate(Duration::from_secs(60), &CancellationToken::new())
        .await
        .unwrap();
    reloaded
        .reload(Duration::from_secs(60), &CancellationToken::new())
        .await
        .unwrap_or_else(|error| panic!("reload failed: {error}\n{}", tail(&reloaded.check_log)));
    let blog_page = get_ok(
        blog_port,
        &format!("localhost:{blog_port}"),
        "/",
        Duration::from_secs(30),
    )
    .await;
    assert!(blog_page.contains("hello from php"), "{blog_page}");
    get_ok(shop_port, &host, "/", Duration::from_secs(10)).await;
    assert_eq!(
        supervisor.status("caddy").unwrap().pid,
        caddy_status.pid,
        "reloaded, not restarted"
    );

    // 6. A Caddyfile Caddy rejects never reaches the running Caddy: validation says no, and the sites stay up.
    let mut broken = reloaded.clone();
    fs::write(&broken.config_file, "this is { not a Caddyfile\n").unwrap();
    broken.reload_command.args.clear();
    let error = broken
        .validate(Duration::from_secs(60), &CancellationToken::new())
        .await
        .unwrap_err();
    eprintln!("a broken Caddyfile: {error}\n{}", tail(&broken.check_log));
    assert_eq!(error.code, luna_core::ErrorCode::ConfigInvalid);
    get_ok(shop_port, &host, "/", Duration::from_secs(10)).await;

    // 7. Stop in the order of the app's Quit: Caddy, then PHP. Ports are released and PID files are gone.
    let begun = Instant::now();
    supervisor.stop("caddy").await.unwrap();
    eprintln!("caddy stopped in {:?}", begun.elapsed());
    assert!(
        begun.elapsed() < Duration::from_secs(10),
        "it was killed after the grace period, not stopped through its admin API"
    );
    for spec in &php.specs {
        supervisor.stop(&spec.id).await.unwrap();
    }
    for port in [shop_port, blog_port, caddy.admin_port]
        .into_iter()
        .chain(php.ports.iter().copied())
    {
        assert!(port_owners(port).is_empty(), "port {port} is free again");
    }
    for id in std::iter::once("caddy").chain(php.specs.iter().map(|s| s.id.as_str())) {
        assert!(!home.run_dir().join(format!("{id}.pid")).exists(), "{id}");
    }
    // The projects were only read.
    assert_eq!(
        fs::read_dir(&shop).unwrap().count(),
        2,
        "nothing was written into the project folder"
    );
    eprintln!("php {} and caddy: ok", php_runtime.version);
}
