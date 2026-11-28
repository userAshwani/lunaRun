//! Opt-in check of the two fixes of a project's database settings against the **real MariaDB** of this platform
//! (FR-PJ-05; D-041). The newest MariaDB with a build for this computer is installed by the real installer, a project
//! whose `.env` names a database and a wrong `DB_PORT` is added to the real store, and the real launcher:
//!
//! 1. says the port is wrong, previews the one line, backs the file up and changes only that line;
//! 2. says the database cannot be checked while the server is down, then (the real service started through the real
//!    controller) that it is missing, creates it with the real client, and sees it exist (a second create changes nothing);
//! 3. refuses a database name that is not plain, and never touches a database that is already there.
//!
//! It downloads about 100 MB, runs third-party programs from the internet and takes a few minutes, so it is ignored by
//! default:
//!
//!     cargo test -p luna-core --test envfix_online -- --ignored --nocapture
//!
//! It proves only the platform it runs on. The home and the project folder have a space and a non-ASCII letter in their
//! paths (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::Arc;

use luna_core::LunaHome;
use luna_core::dbadmin::DatabaseStatus;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::launch::ProjectLauncher;
use luna_core::projects::{DetectionData, ProjectStore, Surroundings};
use luna_core::registry::{Registry, RegistryHolder};
use luna_core::runtimes::Installer;
use luna_core::services::{PortAllocator, ServiceController, Supervisor};
use luna_core::sites::SiteManager;
use luna_core::{ErrorCode, LunaError};
use tempfile::TempDir;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

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

fn code_of(error: &LunaError) -> ErrorCode {
    error.code
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 100 MB and runs a real MariaDB"]
async fn real_mariadb_a_wrong_port_is_fixed_in_one_line_and_a_missing_database_is_created() {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let os = Os::current();
    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
    // MariaDB is not offered on macOS: the project asks for MySQL there.
    let (engine, connection) = if os == Os::Macos {
        ("mysql", "mysql")
    } else {
        ("mariadb", "mariadb")
    };
    install(&home, &jobs, engine).await;

    let folder = tmp.path().join("Projekte größe").join("mon projet é");
    fs::create_dir_all(folder.join("public")).unwrap();
    fs::write(folder.join("public").join("index.php"), "<?php").unwrap();
    fs::write(
        folder.join("composer.json"),
        r#"{"require":{"php":">=8.1"}}"#,
    )
    .unwrap();
    let original = format!(
        "APP_NAME=Shop\r\nDB_CONNECTION={connection}\r\nDB_HOST=127.0.0.1\r\nDB_PORT=1111\r\nDB_DATABASE=café_shop\r\nDB_PASSWORD=\"never read #1\"\r\n"
    );
    // Not plain: it is refused until the .env says a plain name.
    fs::write(folder.join(".env"), &original).unwrap();

    let data = Arc::new(DetectionData::bundled().unwrap());
    let store = ProjectStore::open(&home).unwrap();
    let project = store
        .add(
            &folder,
            &data,
            &Surroundings {
                installed: &[],
                available: &[],
                os,
                defaults: &luna_core::settings::RuntimeDefaults::NONE,
            },
        )
        .unwrap()
        .project;
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
    let controller = ServiceController::new(home.clone(), ports.clone(), supervisor.clone(), os);
    let launcher = ProjectLauncher::new(store, sites, controller.clone(), ports, os);

    // 1. The wrong port: previewed, backed up, one line changed.
    let settings = launcher.database_settings(&project.id).unwrap().unwrap();
    let preview = launcher.env_port_preview(&project.id).unwrap().unwrap();
    assert_eq!(preview.port, settings.port);
    assert_eq!(preview.before.as_deref(), Some("DB_PORT=1111"));
    assert_eq!(preview.after, format!("DB_PORT={}", settings.port));
    assert_eq!(preview.line, 4);
    assert_eq!(fs::read_to_string(folder.join(".env")).unwrap(), original);
    let done = launcher.env_port_apply(&project.id, &preview).unwrap();
    assert_eq!(done.backup, ".env.lunarun-backup");
    assert_eq!(
        fs::read_to_string(folder.join(&done.backup)).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(folder.join(".env")).unwrap(),
        original.replace("DB_PORT=1111", &format!("DB_PORT={}", settings.port))
    );
    assert_eq!(launcher.env_port_preview(&project.id).unwrap(), None);

    // 2. A name with an accented letter is not used in a statement.
    assert_eq!(
        launcher.database_status(&project.id).unwrap(),
        DatabaseStatus::InvalidName
    );
    assert_eq!(
        code_of(&launcher.database_create(&project.id).unwrap_err()),
        ErrorCode::DatabaseNotRunning
    );
    let plain = fs::read_to_string(folder.join(".env"))
        .unwrap()
        .replace("café_shop", "cafe_shop");
    fs::write(folder.join(".env"), plain).unwrap();

    // 3. The server is not running: nothing can be asked of it.
    assert_eq!(
        launcher.database_status(&project.id).unwrap(),
        DatabaseStatus::NotRunning
    );
    assert_eq!(
        code_of(&launcher.database_create(&project.id).unwrap_err()),
        ErrorCode::DatabaseNotRunning
    );

    // 4. The real server, through the real controller.
    controller
        .start(&settings.service, &CancellationToken::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(
        launcher.database_status(&project.id).unwrap(),
        DatabaseStatus::Missing
    );
    let created = launcher.database_create(&project.id).unwrap();
    assert_eq!(
        (created.name.as_str(), created.created),
        ("cafe_shop", true)
    );
    assert_eq!(
        launcher.database_status(&project.id).unwrap(),
        DatabaseStatus::Exists
    );
    let again = launcher.database_create(&project.id).unwrap();
    assert!(!again.created, "a second create changes nothing");

    controller.stop(&settings.service).await.unwrap();
}
