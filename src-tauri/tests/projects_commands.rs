//! The project commands wired into the app: the JSON the page receives and sends, the errors it gets, and that a project
//! bound by hand makes the uninstall dialog list it (and the uninstall itself ask first). The store, detection and
//! resolution have their tests in `luna-core`; this checks the wiring, on Tauri's mock runtime, with a data folder and a
//! project folder that each have a space and a non-ASCII letter.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use luna_core::diagnostics::{Arch, Os};
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::projects::{DetectionData, ProjectPatch, ProjectStore};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::InstallReceipt;
use luna_core::{ErrorCode, LunaHome};
use lunarun_lib::{commands, services};
use serde_json::json;
use tauri::Manager;
use tauri::test::MockRuntime;
use tempfile::TempDir;

struct Bench {
    tmp: TempDir,
    app: tauri::App<MockRuntime>,
    home: LunaHome,
}

/// An app with what `lib.rs` builds for the projects.
fn bench() -> Bench {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let app = tauri::test::mock_app();
    app.manage(home.clone());
    let projects = ProjectStore::open(&home).unwrap();
    services::manage(app.handle(), &home, &projects);
    app.manage(projects);
    app.manage(Arc::new(DetectionData::bundled().unwrap()));
    app.manage(RegistryHolder::with_bundled().unwrap());
    app.manage(JobManager::new(
        tauri::async_runtime::handle().inner().clone(),
        |_| {},
        JobOptions::default(),
    ));
    Bench { tmp, app, home }
}

impl Bench {
    fn folder(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.tmp.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn add(&self, dir: &Path) -> Result<serde_json::Value, luna_core::LunaError> {
        let added = tauri::async_runtime::block_on(commands::project_add(
            dir.to_str().unwrap().to_owned(),
            self.app.state(),
            self.app.state(),
            self.app.state(),
            self.app.state(),
        ))?;
        Ok(serde_json::to_value(added).unwrap())
    }

    fn install_fake(&self, runtime: &str, version: &str) {
        let dir = self.home.runtime_dir(runtime, version).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin").join("program"), "binary").unwrap();
        InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
            .write(&dir)
            .unwrap();
    }
}

#[test]
fn an_added_project_arrives_as_the_json_the_page_reads() {
    let b = bench();
    let dir = b.folder(
        "mon projet é",
        &[
            ("artisan", ""),
            (
                "composer.json",
                r#"{"require":{"php":"^8.2","laravel/framework":"^11"}}"#,
            ),
            ("public/index.php", "<?php"),
        ],
    );
    let added = b.add(&dir).unwrap();

    let project = &added["project"];
    assert_eq!(project["name"], "mon projet é");
    assert_eq!(project["path"], dir.to_str().unwrap());
    assert_eq!(project["docRoot"], "public");
    assert_eq!(project["docRootCustom"], false);
    assert_eq!(project["framework"]["kind"], "laravel");
    assert_eq!(project["bindings"], json!({ "php": null, "node": null }));
    assert!(project["createdAt"].is_u64());
    assert_eq!(
        project["lastDetection"]["hash"],
        added["report"]["inputsHash"]
    );
    assert_eq!(added["report"]["php"]["constraint"], ">=8.2.0 <9.0.0");
    assert_eq!(added["report"]["docRoot"]["path"], "public");
    // Nothing is installed in this app, so PHP is something to install.
    let php = &added["resolution"]["php"];
    assert_eq!(php["recommended"]["needsInstall"], true);
    assert_eq!(php["basis"], "constraint");

    let id = project["id"].as_str().unwrap().to_owned();
    let listed = serde_json::to_value(commands::projects_list(b.app.state()).unwrap()).unwrap();
    assert_eq!(listed, json!([project]));
    let got = commands::project_get(id.clone(), b.app.state()).unwrap();
    assert_eq!(serde_json::to_value(got).unwrap(), *project);

    // Re-scan: the same answer, read again.
    let again = tauri::async_runtime::block_on(commands::project_detect(
        id,
        b.app.state(),
        b.app.state(),
        b.app.state(),
        b.app.state(),
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(again).unwrap()["report"],
        added["report"]
    );
}

#[test]
fn what_the_page_sends_to_change_a_project_is_understood_and_what_is_wrong_is_refused() {
    let b = bench();
    let dir = b.folder("sent", &[("index.php", ""), ("public/index.php", "")]);
    let id = b.add(&dir).unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The exact shapes of src/bindings/ProjectPatch.ts.
    let patch: ProjectPatch = serde_json::from_value(json!({
        "name": "Shop",
        "docRoot": { "kind": "use", "path": "public" },
        "php": { "kind": "use", "binding": { "mode": "line", "value": "8.3" } },
        "node": { "kind": "auto" },
    }))
    .unwrap();
    let updated = tauri::async_runtime::block_on(commands::project_update(
        id.clone(),
        patch,
        b.app.state(),
        b.app.state(),
    ))
    .unwrap();
    let updated = serde_json::to_value(updated).unwrap();
    assert_eq!(updated["name"], "Shop");
    assert_eq!(
        (updated["docRoot"].clone(), updated["docRootCustom"].clone()),
        ("public".into(), true.into())
    );
    assert_eq!(
        updated["bindings"]["php"],
        json!({ "mode": "line", "value": "8.3" })
    );
    // An empty object is a valid patch that changes nothing.
    let nothing: ProjectPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(nothing, ProjectPatch::default());

    let bad: ProjectPatch = serde_json::from_value(json!({
        "php": { "kind": "use", "binding": { "mode": "line", "value": "eight" } }
    }))
    .unwrap();
    let error = tauri::async_runtime::block_on(commands::project_update(
        id,
        bad,
        b.app.state(),
        b.app.state(),
    ))
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details.unwrap()["field"], "php");
}

#[test]
fn errors_reach_the_page_as_typed_codes() {
    let b = bench();
    let dir = b.folder("errors", &[]);
    b.add(&dir).unwrap();
    assert_eq!(b.add(&dir).unwrap_err().code, ErrorCode::ProjectExists);
    assert_eq!(
        b.add(&b.tmp.path().join("missing")).unwrap_err().code,
        ErrorCode::InvalidPath
    );
    assert_eq!(
        b.add(&PathBuf::from("relative")).unwrap_err().code,
        ErrorCode::InvalidPath
    );
    assert_eq!(
        commands::project_get("nope".into(), b.app.state())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        commands::project_remove("nope".into(), b.app.state())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn removing_a_project_leaves_the_folder_and_the_list_empty() {
    let b = bench();
    let dir = b.folder("remove me", &[("composer.json", "{}")]);
    let id = b.add(&dir).unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    commands::project_remove(id, b.app.state()).unwrap();
    assert!(commands::projects_list(b.app.state()).unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.join("composer.json")).unwrap(), "{}");
}

#[test]
fn a_project_bound_by_hand_is_listed_by_the_uninstall_dialog_and_the_uninstall_asks_first() {
    let b = bench();
    b.install_fake("php", "8.3.14");
    let dir = b.folder("bound shop", &[("index.php", "")]);
    let id = b.add(&dir).unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let usage = |version: &str| {
        commands::runtime_usage("php".into(), version.into(), b.app.state()).unwrap()
    };
    assert!(
        usage("8.3.14").bound_projects.is_empty(),
        "nothing is bound by hand yet"
    );

    let bind: ProjectPatch = serde_json::from_value(json!({
        "php": { "kind": "use", "binding": { "mode": "line", "value": "8.3" } }
    }))
    .unwrap();
    tauri::async_runtime::block_on(commands::project_update(
        id.clone(),
        bind,
        b.app.state(),
        b.app.state(),
    ))
    .unwrap();
    assert_eq!(usage("8.3.14").bound_projects, ["bound shop"]);

    let uninstaller = b.app.state::<luna_core::runtimes::Uninstaller>();
    let jobs = b.app.state::<JobManager>();
    let options = luna_core::runtimes::UninstallOptions {
        delete_data: false,
        confirm_bound_projects: false,
    };
    let started = uninstaller.start(&jobs, "php", "8.3.14", options).unwrap();
    let finished = tauri::async_runtime::block_on(jobs.wait(&started.id)).unwrap();
    assert_eq!(
        finished.state,
        JobState::Failed,
        "it asks first: the job fails before it removes anything"
    );
    let refused = finished.error.unwrap();
    assert_eq!(refused.code, ErrorCode::ProjectsBound);
    assert_eq!(refused.details.unwrap()["projects"], json!(["bound shop"]));
    assert!(
        b.home
            .runtime_dir("php", "8.3.14")
            .unwrap()
            .join("bin")
            .is_dir(),
        "still installed"
    );

    // Forgetting the project frees the version.
    commands::project_remove(id, b.app.state()).unwrap();
    assert!(usage("8.3.14").bound_projects.is_empty());
}
