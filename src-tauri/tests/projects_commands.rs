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
use luna_core::launch::{DatabaseSettings, ProjectLauncher};
use luna_core::opener::OpenTarget;
use luna_core::projects::{DetectionData, ProjectPatch, ProjectStore};
use luna_core::registry::RegistryHolder;
use luna_core::runtimes::InstallReceipt;
use luna_core::sites::SiteManager;
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
    let data = Arc::new(DetectionData::bundled().unwrap());
    let registry = RegistryHolder::with_bundled().unwrap();
    services::manage(app.handle(), &home, &projects, &data, &registry);
    app.manage(projects);
    app.manage(data);
    app.manage(registry);
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
        tauri::async_runtime::block_on(commands::project_remove(
            "nope".into(),
            b.app.state(),
            b.app.state()
        ))
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
    tauri::async_runtime::block_on(commands::project_remove(id, b.app.state(), b.app.state()))
        .unwrap();
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
    tauri::async_runtime::block_on(commands::project_remove(id, b.app.state(), b.app.state()))
        .unwrap();
    assert!(usage("8.3.14").bound_projects.is_empty());
}

fn sites_view(b: &Bench) -> serde_json::Value {
    let view = tauri::async_runtime::block_on(commands::sites_list(b.app.state())).unwrap();
    serde_json::to_value(view).unwrap()
}

fn caddyfile(b: &Bench) -> String {
    fs::read_to_string(b.home.config_dir().join("caddy").join("Caddyfile")).unwrap_or_default()
}

#[test]
fn a_project_with_php_installed_is_served_and_the_caddyfile_follows_the_list() {
    let b = bench();
    b.install_fake("php", "8.3.14");
    b.install_fake("caddy", "2.11.7");
    let dir = b.folder(
        "served shop",
        &[
            ("artisan", ""),
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("public/index.php", "<?php"),
        ],
    );
    let added = b.add(&dir).unwrap();
    let id = added["project"]["id"].as_str().unwrap().to_owned();

    let view = sites_view(&b);
    let site = &view["sites"][0];
    assert_eq!(site["projectId"], id.as_str());
    assert_eq!(site["state"], "ready");
    assert_eq!(site["phpVersion"], "8.3.14");
    let port = site["port"].as_u64().unwrap();
    assert_eq!(site["address"], format!("http://127.0.0.1:{port}"));
    assert_eq!(view["webServer"], "stopped", "installed, not running");
    assert_eq!(view["problem"], serde_json::Value::Null);

    // A project is in the Caddyfile once it is started, not before: the project has a site and Caddy does not serve it.
    let served = dir.join("public").display().to_string().replace('\\', "/");
    assert!(!caddyfile(&b).contains(&served), "{}", caddyfile(&b));
    assert_eq!(site["served"], false);
    b.app.state::<SiteManager>().serve(&id);
    tauri::async_runtime::block_on(commands::sites_sync(b.app.state())).unwrap();
    let text = caddyfile(&b);
    assert!(text.contains(&format!("http://localhost:{port}")), "{text}");
    assert!(text.contains(&served), "{text}");

    // Removing the project stops serving it, and the folder is untouched.
    tauri::async_runtime::block_on(commands::project_remove(id, b.app.state(), b.app.state()))
        .unwrap();
    assert!(!caddyfile(&b).contains(&served));
    assert_eq!(sites_view(&b)["sites"], serde_json::json!([]));
    assert!(dir.join("artisan").is_file());
}

#[test]
fn without_php_there_is_no_site_and_the_view_says_so() {
    let b = bench();
    let dir = b.folder(
        "no php",
        &[("composer.json", r#"{"require":{"php":"^8.2"}}"#)],
    );
    b.add(&dir).unwrap();

    let view = sites_view(&b);
    let site = &view["sites"][0];
    assert_eq!(site["state"], "noPhp");
    assert_eq!(
        (site["port"].clone(), site["address"].clone()),
        (serde_json::Value::Null, serde_json::Value::Null)
    );
    assert_eq!(view["webServer"], "notInstalled");
}

#[test]
fn sites_sync_answers_with_the_view_it_leaves_behind() {
    let b = bench();
    b.install_fake("php", "8.3.14");
    b.install_fake("caddy", "2.11.7");
    let dir = b.folder(
        "sync me",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
        ],
    );
    b.add(&dir).unwrap();

    let synced = tauri::async_runtime::block_on(commands::sites_sync(b.app.state())).unwrap();
    let synced = serde_json::to_value(synced).unwrap();
    assert_eq!(synced, sites_view(&b));
    assert_eq!(synced["sites"][0]["state"], "ready");
}

#[test]
fn installing_php_gives_the_project_a_site_without_anyone_asking() {
    // The job sink re-syncs the sites when an install job ends. Here the PHP appears and the sync is asked for the way the
    // sink does it: the sites are planned from what is installed now.
    let b = bench();
    b.install_fake("caddy", "2.11.7");
    let dir = b.folder(
        "late php",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
        ],
    );
    let id = b.add(&dir).unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(sites_view(&b)["sites"][0]["state"], "noPhp");

    b.install_fake("php", "8.3.14");
    b.app.state::<SiteManager>().serve(&id);
    tauri::async_runtime::block_on(commands::sites_sync(b.app.state())).unwrap();
    assert_eq!(sites_view(&b)["sites"][0]["state"], "ready");
    assert!(caddyfile(&b).contains("php_fastcgi"));
}

// ---------------------------------------------------------------------------------------------
// Starting and stopping a project, and the quick actions
// ---------------------------------------------------------------------------------------------

fn project_id(added: &serde_json::Value) -> String {
    added["project"]["id"].as_str().unwrap().to_owned()
}

fn wait_job(b: &Bench, started: &luna_core::jobs::StartedJob) -> luna_core::jobs::JobInfo {
    tauri::async_runtime::block_on(b.app.state::<JobManager>().wait(&started.id)).unwrap()
}

#[test]
fn the_new_commands_say_not_found_for_a_project_that_is_not_there() {
    let b = bench();
    let codes = [
        commands::project_start("nope".into(), b.app.state(), b.app.state())
            .unwrap_err()
            .code,
        commands::project_stop("nope".into(), b.app.state(), b.app.state())
            .unwrap_err()
            .code,
        // Nothing is opened for a project that does not exist: the id is checked before anything is run.
        tauri::async_runtime::block_on(commands::project_open(
            "nope".into(),
            OpenTarget::Folder,
            b.app.state(),
            b.app.state(),
            b.app.state(),
        ))
        .unwrap_err()
        .code,
        tauri::async_runtime::block_on(commands::project_database("nope".into(), b.app.state()))
            .unwrap_err()
            .code,
    ];
    assert_eq!(codes, [ErrorCode::NotFound; 4]);
}

#[test]
fn starting_a_project_whose_php_is_not_installed_fails_the_job_with_no_site() {
    let b = bench();
    b.install_fake("caddy", "2.11.7");
    let dir = b.folder(
        "no php yet",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
        ],
    );
    let id = project_id(&b.add(&dir).unwrap());

    let started = commands::project_start(id.clone(), b.app.state(), b.app.state()).unwrap();
    let info = wait_job(&b, &started);

    assert_eq!(info.key, format!("project:{id}"));
    assert_eq!(info.state, JobState::Failed);
    let error = info.error.unwrap();
    assert_eq!(error.code, ErrorCode::NoSite);
    assert_eq!(error.details.unwrap()["reason"], "php");
    // Nothing was started, so the project is not served.
    assert_eq!(sites_view(&b)["sites"][0]["served"], false);
}

#[test]
fn stopping_a_project_that_was_never_started_succeeds() {
    let b = bench();
    let dir = b.folder("idle", &[("composer.json", "{}"), ("index.php", "<?php")]);
    let id = project_id(&b.add(&dir).unwrap());

    let started = commands::project_stop(id, b.app.state(), b.app.state()).unwrap();

    assert_eq!(wait_job(&b, &started).state, JobState::Succeeded);
}

#[test]
fn the_browser_cannot_be_opened_for_a_project_without_a_site() {
    let b = bench();
    let dir = b.folder(
        "no site",
        &[("composer.json", "{}"), ("index.php", "<?php")],
    );
    let id = project_id(&b.add(&dir).unwrap());

    let error = tauri::async_runtime::block_on(commands::project_open(
        id,
        OpenTarget::Browser,
        b.app.state(),
        b.app.state(),
        b.app.state(),
    ))
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::NoSite, "{}", error.message);
}

#[test]
fn a_terminal_needs_the_php_of_the_project_and_nothing_is_written_without_it() {
    let b = bench();
    let dir = b.folder(
        "no php for a terminal",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
        ],
    );
    let id = project_id(&b.add(&dir).unwrap());

    let error = tauri::async_runtime::block_on(commands::project_open(
        id.clone(),
        OpenTarget::Terminal,
        b.app.state(),
        b.app.state(),
        b.app.state(),
    ))
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::NoSite, "{}", error.message);
    assert_eq!(error.details.unwrap()["reason"], "php");
    let home = b.app.state::<luna_core::LunaHome>();
    assert!(!home.run_dir().join("terminals").exists());
    assert!(!home.config_dir().join("shims").exists());
}

#[test]
fn a_terminal_for_a_project_that_is_not_there_says_not_found() {
    let b = bench();
    let error = tauri::async_runtime::block_on(commands::project_open(
        "nope".into(),
        OpenTarget::Terminal,
        b.app.state(),
        b.app.state(),
        b.app.state(),
    ))
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn the_database_settings_arrive_as_the_json_the_page_reads() {
    let b = bench();
    b.install_fake("php", "8.3.14");
    let dir = b.folder(
        "with db",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
            (
                ".env",
                "DB_CONNECTION=mysql\nDB_DATABASE=shop\nDB_PASSWORD=never-in-a-report\n",
            ),
        ],
    );
    let added = b.add(&dir).unwrap();
    let id = project_id(&added);
    let database = |b: &Bench| {
        tauri::async_runtime::block_on(commands::project_database(id.clone(), b.app.state()))
            .unwrap()
    };
    assert_eq!(database(&b), None, "the runtime is not installed");

    // Install the MySQL line the registry suggests for this project.
    let project = b.app.state::<ProjectStore>().get(&id).unwrap();
    let (_, resolution) = b.app.state::<SiteManager>().evaluate(&project).unwrap();
    let line = resolution.database.unwrap().line.unwrap();
    b.install_fake("mysql", &format!("{line}.13"));

    let settings: DatabaseSettings = database(&b).unwrap();
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["service"], format!("mysql@{line}"));
    assert_eq!(json["runtime"], "mysql");
    assert_eq!(json["host"], "127.0.0.1");
    assert_eq!(json["username"], "root");
    assert_eq!(json["database"], "shop");
    assert!(json["port"].as_u64().unwrap() >= 3306);
    assert!(
        !json.to_string().contains("never-in-a-report") && json.get("password").is_none(),
        "no password, of the .env file or any other: {json}"
    );
}

#[test]
fn the_launcher_is_part_of_the_apps_state() {
    let b = bench();
    let _ = b.app.state::<ProjectLauncher>();
}

// ---------------------------------------------------------------------------------------------
// The fixes of the database settings (D-041)
// ---------------------------------------------------------------------------------------------

macro_rules! run {
    ($future:expr) => {
        tauri::async_runtime::block_on($future)
    };
}

fn made_up_change() -> luna_core::projects::envfix::PortChange {
    // What a page could send: the JSON the preview has, as it arrives over the bridge.
    serde_json::from_value(serde_json::json!({
        "port": 3306,
        "line": 1,
        "before": "DB_PORT=1111",
        "after": "DB_PORT=3306",
        "backup": ".env.lunarun-backup"
    }))
    .unwrap()
}

#[test]
fn the_database_fixes_say_not_found_for_a_project_that_is_not_there() {
    let b = bench();
    let codes = [
        run!(commands::project_env_port_preview(
            "nope".into(),
            b.app.state()
        ))
        .unwrap_err()
        .code,
        run!(commands::project_env_port_apply(
            "nope".into(),
            made_up_change(),
            b.app.state(),
        ))
        .unwrap_err()
        .code,
        run!(commands::project_database_status(
            "nope".into(),
            b.app.state()
        ))
        .unwrap_err()
        .code,
        run!(commands::project_database_create(
            "nope".into(),
            b.app.state()
        ))
        .unwrap_err()
        .code,
    ];
    assert_eq!(codes, [ErrorCode::NotFound; 4]);
}

#[test]
fn without_a_database_lunarun_runs_there_is_nothing_to_fix_and_nothing_is_written() {
    let b = bench();
    let env = "DB_CONNECTION=mysql\nDB_PORT=1111\nDB_DATABASE=shop\nDB_PASSWORD=never-read\n";
    let dir = b.folder(
        "no database yet",
        &[
            ("composer.json", r#"{"require":{"php":"^8.2"}}"#),
            ("index.php", "<?php"),
            (".env", env),
        ],
    );
    let id = project_id(&b.add(&dir).unwrap());

    // MySQL is not installed: lunaRun runs no database for this project, so there is no port to compare with.
    assert_eq!(
        run!(commands::project_env_port_preview(
            id.clone(),
            b.app.state()
        ))
        .unwrap(),
        None
    );
    assert_eq!(
        serde_json::to_value(
            run!(commands::project_database_status(id.clone(), b.app.state())).unwrap()
        )
        .unwrap(),
        "notApplicable"
    );
    assert_eq!(
        run!(commands::project_database_create(id.clone(), b.app.state()))
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    // A change the page made up is not applied, and the file is as it was, with no backup next to it.
    let err = run!(commands::project_env_port_apply(
        id,
        made_up_change(),
        b.app.state(),
    ))
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::EnvChanged, "{}", err.message);
    assert_eq!(std::fs::read_to_string(dir.join(".env")).unwrap(), env);
    assert!(!dir.join(".env.lunarun-backup").exists());
}

#[test]
fn the_preview_and_the_results_arrive_as_the_json_the_page_reads() {
    let preview = made_up_change();
    assert_eq!(
        serde_json::to_value(&preview).unwrap(),
        serde_json::json!({
            "port": 3306,
            "line": 1,
            "before": "DB_PORT=1111",
            "after": "DB_PORT=3306",
            "backup": ".env.lunarun-backup"
        })
    );
    assert_eq!(
        serde_json::to_value(luna_core::projects::envfix::EnvFixApplied {
            backup: ".env.lunarun-backup".into()
        })
        .unwrap(),
        serde_json::json!({ "backup": ".env.lunarun-backup" })
    );
    assert_eq!(
        serde_json::to_value(luna_core::dbadmin::DatabaseCreated {
            name: "shop".into(),
            created: true
        })
        .unwrap(),
        serde_json::json!({ "name": "shop", "created": true })
    );
    let codes = [
        (ErrorCode::EnvChanged, "ENV_CHANGED"),
        (ErrorCode::DatabaseNotRunning, "DATABASE_NOT_RUNNING"),
        (ErrorCode::DatabaseCommandFailed, "DATABASE_COMMAND_FAILED"),
    ];
    for (code, text) in codes {
        assert_eq!(serde_json::to_value(code).unwrap(), text);
    }
}

// ---------------------------------------------------------------------------------------------
// Settings (D-045)
// ---------------------------------------------------------------------------------------------

#[test]
fn the_settings_are_read_checked_and_stored_as_the_page_sends_them() {
    let b = bench();
    let view = commands::settings_get(b.app.state()).unwrap();
    assert_eq!(view.current, view.defaults);
    assert!(view.default_registry_url.starts_with("https://"));

    // What the page sends, as JSON.
    let mut sent: luna_core::settings::Settings =
        serde_json::from_value(serde_json::to_value(&view.current).unwrap()).unwrap();
    sent.default_php_line = Some("8.4".to_owned());
    let saved = commands::settings_set(sent.clone(), b.app.state()).unwrap();
    assert_eq!(saved.current.default_php_line.as_deref(), Some("8.4"));
    assert_eq!(
        commands::settings_get(b.app.state()).unwrap().current,
        saved.current
    );

    // A refused value names its field and changes nothing.
    sent.registry_url = Some("http://plain.example/r.json".to_owned());
    let err = commands::settings_set(sent, b.app.state()).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidUrl);
    assert_eq!(err.details.unwrap()["field"], "registryUrl");
    assert_eq!(
        commands::settings_get(b.app.state()).unwrap().current,
        saved.current
    );
}
