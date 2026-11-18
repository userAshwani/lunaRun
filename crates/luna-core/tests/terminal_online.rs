//! Opt-in check of the terminal of a project against the **real PHP, Composer and Node.js** of this platform (FR-PJ-08;
//! D-040). The newest of each with a build for this computer is installed by the real installer, a project is added to
//! the real store, the real `SiteManager` and `terminal::prepare` build the plan, its files are written, and a shell is
//! started **with the environment the plan gives the terminal**. In that shell `php`, `composer` and `node` must be the
//! installed ones (found first on `PATH`, not whatever the machine has), PHP must read the `php.ini` lunaRun wrote, and the
//! shell must start in the project folder with `LUNA_PROJECT` set.
//!
//! It downloads about 60 MB and runs third-party programs from the internet, so it is ignored by default:
//!
//!     cargo test -p luna-core --test terminal_online -- --ignored --nocapture
//!
//! It proves only the platform it runs on. The home and the project folder have a space and a non-ASCII letter in their
//! paths (NFR-07). The terminal window itself is not opened: a window cannot be asserted on, and the real app window is
//! checked by hand (TRACKER section 7).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::projects::{DetectionData, ProjectStore, Surroundings};
use luna_core::registry::{Registry, RegistryHolder};
use luna_core::runtimes::Installer;
use luna_core::services::{PortAllocator, Supervisor};
use luna_core::sites::SiteManager;
use luna_core::terminal;
use tempfile::TempDir;
use tokio::runtime::Handle;

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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 60 MB and runs a real PHP, Composer and Node.js"]
async fn the_terminal_of_a_project_runs_its_own_php_composer_and_node() {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let os = Os::current();

    let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
    let php_version = install(&home, &jobs, "php").await;
    let composer_version = install(&home, &jobs, "composer").await;
    let node_version = install(&home, &jobs, "node").await;

    let folder = tmp.path().join("Projekte größe").join("mon projet é");
    fs::create_dir_all(folder.join("public")).unwrap();
    fs::write(folder.join("public").join("index.php"), "<?php").unwrap();
    fs::write(
        folder.join("composer.json"),
        r#"{"require":{"php":">=8.1"}}"#,
    )
    .unwrap();
    fs::write(
        folder.join("package.json"),
        r#"{"engines":{"node":">=18"}}"#,
    )
    .unwrap();

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
            },
        )
        .unwrap()
        .project;
    let supervisor = Supervisor::new(Handle::current(), home.clone(), |_| {});
    let sites = SiteManager::new(
        home.clone(),
        store.clone(),
        Arc::clone(&data),
        RegistryHolder::with_bundled().unwrap(),
        PortAllocator::new(home.clone()),
        supervisor,
        os,
    );

    let plan = terminal::prepare(&home, os, &store, &sites, &project.id).unwrap();
    terminal::write_files(&plan.files).unwrap();
    for file in &plan.files {
        assert!(file.path.starts_with(home.root()), "{:?}", file.path);
        assert!(file.path.is_file());
    }

    // What the terminal would run, in a shell that exits: the same environment, the same folder.
    let report = "php -r \"echo PHP_VERSION, PHP_EOL, php_ini_loaded_file(), PHP_EOL;\"\ncomposer --version --no-ansi\nnode --version\n";
    let mut shell = if os == Os::Windows {
        let mut command = Command::new("powershell.exe");
        command
            .args(["-NoLogo", "-NoProfile", "-Command", "-"])
            .envs(plan.command.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&folder);
        command
    } else {
        let script = plan
            .files
            .iter()
            .find(|f| f.executable && f.path.extension().is_some())
            .unwrap();
        let mut command = Command::new("sh");
        command.arg(&script.path).env("SHELL", "/bin/sh");
        command
    }
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    let mut input = shell.stdin.take().unwrap();
    // PowerShell answers in the console's code page; ask for UTF-8 so the paths with accented letters can be compared.
    let mut text = if os == Os::Windows {
        String::from(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
",
        )
    } else {
        String::new()
    };
    text.push_str(report);
    if os == Os::Windows {
        text.push_str("Write-Output \"project=$env:LUNA_PROJECT\"\nWrite-Output \"cwd=$PWD\"\n");
    } else {
        text.push_str("echo \"project=$LUNA_PROJECT\"\necho \"cwd=$PWD\"\n");
    }
    text.push_str("exit\n");
    input.write_all(text.as_bytes()).unwrap();
    drop(input);
    let output = shell.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&output.stdout).into_owned();
    eprintln!("{said}\n{}", String::from_utf8_lossy(&output.stderr));

    assert!(said.contains(&php_version), "php {php_version}: {said}");
    assert!(
        said.contains(&format!("Composer version {composer_version}")),
        "composer {composer_version}: {said}"
    );
    assert!(
        said.contains(&format!("v{node_version}")),
        "node {node_version}: {said}"
    );
    // Only the part after the home folder is compared: PHP on Windows prints a `ü` of the home folder in the console's
    // code page (a known problem with a non-ASCII home, see TRACKER known issues), so the whole path would differ.
    let ini = std::path::Path::new("config")
        .join("php")
        .join(&php_version)
        .join("php.ini");
    assert!(
        said.contains(&ini.display().to_string()),
        "PHP did not read the php.ini lunaRun wrote (…{}): {said}",
        ini.display()
    );
    assert!(said.contains(&format!("project={}", project.id)), "{said}");
    assert!(said.contains("mon projet é"), "{said}");
}
