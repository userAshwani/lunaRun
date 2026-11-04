//! Uninstalling runtimes. Every data folder has a space and non-ASCII characters in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use luna_core::LunaError;
use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::error::ErrorCode;
use luna_core::jobs::{JobManager, JobOptions, JobState};
use luna_core::registry::Registry;
use luna_core::runtimes::{
    InstallReceipt, Installer, RECEIPT_FILE, RuntimeUsage, UninstallOptions, UninstallReport,
    Uninstaller, UsageReport, cleanup_incomplete, list_installed, php_config_paths,
    write_php_config,
};
use tempfile::TempDir;
use tokio::runtime::Handle;

struct Bench {
    _tmp: TempDir,
    home: LunaHome,
    jobs: JobManager,
}

impl Bench {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        let jobs = JobManager::new(Handle::current(), |_| {}, JobOptions::default());
        Self {
            _tmp: tmp,
            home,
            jobs,
        }
    }

    /// An installed runtime: its folder with a program-like file inside and a valid receipt.
    fn install(&self, runtime: &str, version: &str) -> PathBuf {
        let dir = self.home.runtime_dir(runtime, version).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin").join("program"), "binary").unwrap();
        InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
            .write(&dir)
            .unwrap();
        dir
    }

    fn installed(&self) -> Vec<String> {
        list_installed(&self.home)
            .unwrap()
            .iter()
            .map(|i| format!("{} {}", i.runtime, i.version))
            .collect()
    }

    /// Run an uninstall as a job and return its result.
    async fn uninstall(
        &self,
        uninstaller: &Uninstaller,
        runtime: &str,
        version: &str,
        options: UninstallOptions,
    ) -> Result<UninstallReport, LunaError> {
        let slot: Arc<Mutex<Option<Result<UninstallReport, LunaError>>>> = Arc::default();
        let out = Arc::clone(&slot);
        let (uninstaller, runtime_id, version_text) =
            (uninstaller.clone(), runtime.to_owned(), version.to_owned());
        let started = self.jobs.start(
            &format!("direct-{runtime}@{version}-{}", slot_id()),
            "test",
            move |ctx| async move {
                let result = uninstaller
                    .uninstall(&runtime_id, &version_text, options, &ctx)
                    .await;
                let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
                *out.lock().unwrap() = Some(result);
                outcome
            },
        );
        self.jobs.wait(&started.id).await.unwrap();
        slot.lock().unwrap().take().unwrap()
    }

    fn names(&self, runtime: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.home.runtimes_dir().join(runtime))
            .map(|dir| {
                dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

fn slot_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn plain() -> UninstallOptions {
    UninstallOptions::default()
}

fn usage(running: &[&str], projects: &[&str]) -> RuntimeUsage {
    RuntimeUsage {
        running_services: running.iter().map(|s| (*s).to_owned()).collect(),
        bound_projects: projects.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// `data/<runtime>-<line>` with a file in it, as a database would leave.
fn data_dir(bench: &Bench, name: &str) -> PathBuf {
    let dir = bench.home.data_dir().join(name);
    fs::create_dir_all(dir.join("mysql")).unwrap();
    fs::write(dir.join("mysql").join("user.MYD"), "the user's databases").unwrap();
    dir
}

// ---------------------------------------------------------------------------------------------
// The basics
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn uninstalling_removes_the_folder_and_leaves_nothing_behind() {
    let bench = Bench::new();
    let dir = bench.install("node", "24.22.2");
    let uninstaller = Uninstaller::new(bench.home.clone());

    let report = bench
        .uninstall(&uninstaller, "node", "24.22.2", plain())
        .await
        .unwrap();

    assert_eq!(report.removed_dir, dir);
    assert!(!dir.exists());
    assert!(bench.installed().is_empty());
    assert_eq!(
        bench.names("node"),
        Vec::<String>::new(),
        "no .trash folder either"
    );
    assert!(report.removed_config.is_empty() && report.leftover.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_chosen_version_goes_and_the_others_stay_untouched() {
    let bench = Bench::new();
    bench.install("php", "8.3.14");
    let keep = bench.install("php", "8.4.2");
    bench.install("node", "24.22.2");
    let uninstaller = Uninstaller::new(bench.home.clone());

    bench
        .uninstall(&uninstaller, "php", "8.3.14", plain())
        .await
        .unwrap();

    assert_eq!(bench.installed(), ["node 24.22.2", "php 8.4.2"]);
    assert!(keep.join("bin").join("program").is_file());
    assert!(keep.join(RECEIPT_FILE).is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_version_that_is_not_installed_is_not_found_and_nothing_is_touched() {
    let bench = Bench::new();
    bench.install("php", "8.4.2");
    // A folder with the right name but no receipt is not an installed runtime, so it is not removed.
    let stray = bench.home.runtime_dir("php", "8.3.14").unwrap();
    fs::create_dir_all(&stray).unwrap();
    fs::write(stray.join("keep.txt"), "x").unwrap();
    let uninstaller = Uninstaller::new(bench.home.clone());

    for version in ["8.3.14", "9.9.9"] {
        let err = bench
            .uninstall(&uninstaller, "php", version, plain())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound, "{version}");
    }
    let err = bench
        .uninstall(&uninstaller, "ruby", "3.3.0", plain())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(stray.join("keep.txt").is_file());
    assert_eq!(bench.installed(), ["php 8.4.2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_without_an_adapter_is_still_removed() {
    let bench = Bench::new();
    let dir = bench.install("ruby", "3.3.0");
    let uninstaller = Uninstaller::new(bench.home.clone());

    bench
        .uninstall(&uninstaller, "ruby", "3.3.0", plain())
        .await
        .unwrap();

    assert!(!dir.exists());
}

// ---------------------------------------------------------------------------------------------
// What belongs to the version
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn php_takes_its_own_config_with_it_and_leaves_the_shared_bundle_and_other_versions() {
    let bench = Bench::new();
    bench.install("php", "8.3.14");
    bench.install("php", "8.4.2");
    for version in ["8.3.14", "8.4.2"] {
        let paths = php_config_paths(&bench.home, version).unwrap();
        write_php_config(&paths, &format!("; ini of {version}\n")).unwrap();
        fs::write(&paths.user_ini, format!("memory_limit = {version}\n")).unwrap();
    }
    let gone = php_config_paths(&bench.home, "8.3.14").unwrap();
    let kept = php_config_paths(&bench.home, "8.4.2").unwrap();
    let uninstaller = Uninstaller::new(bench.home.clone());

    let report = bench
        .uninstall(&uninstaller, "php", "8.3.14", plain())
        .await
        .unwrap();

    assert_eq!(report.removed_config, std::slice::from_ref(&gone.dir));
    assert!(
        !gone.dir.exists(),
        "its php.ini and its 99-user.ini go with it"
    );
    assert!(kept.ini.is_file() && kept.user_ini.is_file());
    assert_eq!(
        fs::read_to_string(&kept.user_ini).unwrap(),
        "memory_limit = 8.4.2\n"
    );
    assert!(
        kept.cacert.is_file(),
        "the CA bundle is shared by every version"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn database_data_is_kept_unless_the_user_asks() {
    let bench = Bench::new();
    bench.install("mysql", "8.4.13");
    bench.install("mariadb", "11.8.9");
    let mysql_data = data_dir(&bench, "mysql-8.4");
    let mariadb_data = data_dir(&bench, "mariadb-11.8");
    let other_line = data_dir(&bench, "mysql-9.7");
    let uninstaller = Uninstaller::new(bench.home.clone());

    // The default keeps the databases and says where they are.
    let report = bench
        .uninstall(&uninstaller, "mysql", "8.4.13", plain())
        .await
        .unwrap();
    assert_eq!(report.kept_data, std::slice::from_ref(&mysql_data));
    assert!(report.removed_data.is_empty());
    assert!(
        mysql_data.join("mysql").join("user.MYD").is_file(),
        "the data is intact"
    );
    assert!(
        bench.installed() == ["mariadb 11.8.9"],
        "the program itself is gone"
    );

    // Asked explicitly, and only for this runtime's own line.
    let asked = UninstallOptions {
        delete_data: true,
        ..plain()
    };
    let report = bench
        .uninstall(&uninstaller, "mariadb", "11.8.9", asked)
        .await
        .unwrap();
    assert_eq!(report.removed_data, std::slice::from_ref(&mariadb_data));
    assert!(!mariadb_data.exists());
    assert!(
        mysql_data.exists(),
        "another runtime's data is never touched"
    );
    assert!(other_line.exists(), "nor another line's");
}

#[tokio::test(flavor = "multi_thread")]
async fn data_shared_by_two_patch_versions_survives_until_the_last_one_goes() {
    let bench = Bench::new();
    bench.install("mysql", "8.4.12");
    bench.install("mysql", "8.4.13");
    bench.install("mysql", "9.7.4");
    let shared = data_dir(&bench, "mysql-8.4");
    let uninstaller = Uninstaller::new(bench.home.clone());
    let asked = UninstallOptions {
        delete_data: true,
        ..plain()
    };

    // 8.4.12 is still installed, so the 8.4 data stays even though deletion was asked for.
    let report = bench
        .uninstall(&uninstaller, "mysql", "8.4.13", asked)
        .await
        .unwrap();
    assert_eq!(report.kept_data, std::slice::from_ref(&shared));
    assert!(report.removed_data.is_empty());
    assert!(shared.exists());

    // The last 8.4 version takes the data with it, 9.7 is unrelated.
    let report = bench
        .uninstall(&uninstaller, "mysql", "8.4.12", asked)
        .await
        .unwrap();
    assert_eq!(report.removed_data, std::slice::from_ref(&shared));
    assert!(!shared.exists());
    assert_eq!(bench.installed(), ["mysql 9.7.4"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn another_runtime_with_a_similar_version_is_not_a_sibling_that_shares_the_data() {
    let bench = Bench::new();
    bench.install("mysql", "8.4.13");
    // A different runtime that happens to be at 8.4.x: it has its own data folder and shares nothing.
    bench.install("mariadb", "8.4.99");
    let data = data_dir(&bench, "mysql-8.4");
    let uninstaller = Uninstaller::new(bench.home.clone());
    let asked = UninstallOptions {
        delete_data: true,
        ..plain()
    };

    let report = bench
        .uninstall(&uninstaller, "mysql", "8.4.13", asked)
        .await
        .unwrap();

    assert_eq!(report.removed_data, std::slice::from_ref(&data));
    assert!(!data.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_data_folder_that_does_not_exist_is_not_mentioned() {
    let bench = Bench::new();
    bench.install("mysql", "8.4.13");
    let uninstaller = Uninstaller::new(bench.home.clone());
    let asked = UninstallOptions {
        delete_data: true,
        ..plain()
    };

    let report = bench
        .uninstall(&uninstaller, "mysql", "8.4.13", asked)
        .await
        .unwrap();

    assert!(report.kept_data.is_empty() && report.removed_data.is_empty());
}

// ---------------------------------------------------------------------------------------------
// In use, and bound projects
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_running_service_stops_the_uninstall_and_nothing_changes() {
    let bench = Bench::new();
    let dir = bench.install("mysql", "8.4.13");
    let data = data_dir(&bench, "mysql-8.4");
    let uninstaller = Uninstaller::new(bench.home.clone())
        .with_usage(Arc::new(|_, _| Ok(usage(&["mysql@8.4.13", "caddy"], &[]))));
    let asked = UninstallOptions {
        delete_data: true,
        confirm_bound_projects: true,
    };

    let err = bench
        .uninstall(&uninstaller, "mysql", "8.4.13", asked)
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::RuntimeInUse);
    assert_eq!(
        err.details.unwrap()["services"],
        serde_json::json!(["mysql@8.4.13", "caddy"])
    );
    assert!(dir.join("bin").join("program").is_file());
    assert!(data.exists());
    assert_eq!(bench.installed(), ["mysql 8.4.13"]);
    assert_eq!(bench.names("mysql"), ["8.4.13"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn bound_projects_need_an_explicit_confirmation() {
    let bench = Bench::new();
    let dir = bench.install("php", "8.3.14");
    let uninstaller = Uninstaller::new(bench.home.clone())
        .with_usage(Arc::new(|_, _| Ok(usage(&[], &["shop", "blog"]))));

    let err = bench
        .uninstall(&uninstaller, "php", "8.3.14", plain())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ProjectsBound);
    assert_eq!(
        err.details.unwrap()["projects"],
        serde_json::json!(["shop", "blog"])
    );
    assert!(dir.exists(), "refused before anything was moved");

    let confirmed = UninstallOptions {
        confirm_bound_projects: true,
        ..plain()
    };
    bench
        .uninstall(&uninstaller, "php", "8.3.14", confirmed)
        .await
        .unwrap();
    assert!(!dir.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_usage_for_the_confirmation_dialog_can_be_asked_without_uninstalling() {
    let bench = Bench::new();
    bench.install("php", "8.3.14");
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let record = Arc::clone(&seen);
    let uninstaller = Uninstaller::new(bench.home.clone()).with_usage(Arc::new(move |r, v| {
        record.lock().unwrap().push((r.to_owned(), v.to_owned()));
        Ok(usage(&["php-fpm@8.3.14"], &["shop"]))
    }));

    let found = uninstaller.usage("php", "8.3.14").unwrap();

    assert_eq!(found, usage(&["php-fpm@8.3.14"], &["shop"]));
    assert_eq!(
        *seen.lock().unwrap(),
        [("php".to_owned(), "8.3.14".to_owned())]
    );
    assert_eq!(bench.installed(), ["php 8.3.14"], "asking changes nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_usage_report_lists_users_and_the_database_folders_without_changing_anything() {
    let bench = Bench::new();
    bench.install("mysql", "8.4.12");
    bench.install("mysql", "8.4.13");
    bench.install("php", "8.3.14");
    let shared = data_dir(&bench, "mysql-8.4");
    let uninstaller = Uninstaller::new(bench.home.clone())
        .with_usage(Arc::new(|_, _| Ok(usage(&["mysql"], &["shop", "blog"]))));

    // Two patch versions of one line share the folder: deleting it would be refused for the first.
    let report = uninstaller.usage_report("mysql", "8.4.13").unwrap();
    assert_eq!(
        report,
        UsageReport {
            running_services: vec!["mysql".into()],
            bound_projects: vec!["shop".into(), "blog".into()],
            data_dirs: vec![shared.display().to_string()],
            data_shared: true,
        }
    );

    // Alone on its line, the folder would go with it.
    fs::remove_dir_all(bench.home.runtime_dir("mysql", "8.4.12").unwrap()).unwrap();
    let report = uninstaller.usage_report("mysql", "8.4.13").unwrap();
    assert_eq!(report.data_dirs, [shared.display().to_string()]);
    assert!(!report.data_shared);

    // A runtime without databases, and a version that is not installed.
    let report = uninstaller.usage_report("php", "8.3.14").unwrap();
    assert!(report.data_dirs.is_empty() && !report.data_shared);
    let missing = uninstaller.usage_report("php", "9.9.9").unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);

    assert!(shared.exists(), "asking changes nothing");
    assert_eq!(bench.installed(), ["mysql 8.4.13", "php 8.3.14"]);
}

#[test]
fn the_dialog_and_the_commands_get_camel_case_json() {
    let report = UsageReport {
        running_services: vec![],
        bound_projects: vec!["shop".into()],
        data_dirs: vec![r"D:\data".into()],
        data_shared: false,
    };
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["boundProjects"][0], "shop");
    assert_eq!(value["dataShared"], false);
    assert!(value["runningServices"].as_array().unwrap().is_empty());
    assert!(value["dataDirs"].is_array());

    let started = serde_json::to_value(luna_core::jobs::StartedJob {
        id: "job-3".into(),
        already_running: true,
    })
    .unwrap();
    assert_eq!(started["id"], "job-3");
    assert_eq!(started["alreadyRunning"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_usage_check_stops_the_uninstall() {
    let bench = Bench::new();
    let dir = bench.install("php", "8.3.14");
    let uninstaller = Uninstaller::new(bench.home.clone()).with_usage(Arc::new(|_, _| {
        Err(LunaError::new(
            ErrorCode::Internal,
            "the supervisor is not answering",
        ))
    }));

    let err = bench
        .uninstall(&uninstaller, "php", "8.3.14", plain())
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::Internal);
    assert!(
        dir.exists(),
        "when we cannot tell whether it is in use, we do not remove it"
    );
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_program_still_open_in_the_folder_blocks_the_uninstall_without_changing_anything() {
    use std::os::windows::fs::OpenOptionsExt;

    let bench = Bench::new();
    let dir = bench.install("node", "24.22.2");
    let uninstaller = Uninstaller::new(bench.home.clone());
    // Like a running node.exe: a file inside the folder is open and nobody else may touch it.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.join("bin").join("program"))
        .unwrap();

    let err = bench
        .uninstall(&uninstaller, "node", "24.22.2", plain())
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::RuntimeInUse, "{}", err.message);
    assert_eq!(bench.installed(), ["node 24.22.2"], "still installed");
    assert!(dir.join("bin").join("program").is_file());
    assert_eq!(bench.names("node"), ["24.22.2"], "no half-moved folder");

    // Once the program is closed, the same uninstall goes through.
    drop(held);
    bench
        .uninstall(&uninstaller, "node", "24.22.2", plain())
        .await
        .unwrap();
    assert!(bench.installed().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Safety and recovery
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_symlink_inside_the_runtime_folder_is_unlinked_never_followed() {
    let bench = Bench::new();
    let dir = bench.install("node", "24.22.2");
    let outside = bench._tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("precious.txt"), "do not delete me").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("link-to-outside")).unwrap();
    let uninstaller = Uninstaller::new(bench.home.clone());

    bench
        .uninstall(&uninstaller, "node", "24.22.2", plain())
        .await
        .unwrap();

    assert!(!dir.exists());
    assert_eq!(
        fs::read_to_string(outside.join("precious.txt")).unwrap(),
        "do not delete me"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trash_folder_left_by_a_crash_is_never_an_install_and_startup_cleanup_removes_it() {
    let bench = Bench::new();
    bench.install("node", "24.22.2");
    // What an uninstall that died after the rename leaves: a hidden folder that still has the receipt in it.
    let trash = bench.home.runtimes_dir().join("node").join(".trash-job-9");
    fs::create_dir_all(trash.join("deep")).unwrap();
    fs::write(trash.join("deep").join("file"), "x").unwrap();
    InstallReceipt::new("node", "99.0.0", Os::Windows, Arch::X86_64, false, &[])
        .write(&trash)
        .unwrap();

    assert_eq!(bench.installed(), ["node 24.22.2"], "never listed");

    let removed = cleanup_incomplete(&bench.home).unwrap();
    assert_eq!(removed, std::slice::from_ref(&trash));
    assert!(!trash.exists());
    assert_eq!(
        bench.installed(),
        ["node 24.22.2"],
        "the real install is untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_the_point_of_no_return_removes_nothing() {
    let bench = Bench::new();
    let dir = bench.install("node", "24.22.2");
    let (release, wait) = mpsc::channel::<()>();
    let uninstaller = Uninstaller::new(bench.home.clone()).with_usage({
        let wait = Mutex::new(wait);
        Arc::new(move |_, _| {
            wait.lock().unwrap().recv().ok();
            Ok(RuntimeUsage::default())
        })
    });

    let started = uninstaller
        .start(&bench.jobs, "node", "24.22.2", plain())
        .unwrap();
    while bench
        .jobs
        .get(&started.id)
        .is_none_or(|j| j.phase != "preparing")
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    bench.jobs.cancel(&started.id).unwrap();
    release.send(()).unwrap();
    let info = bench.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Cancelled);
    assert!(dir.exists());
    assert_eq!(bench.installed(), ["node 24.22.2"]);
}

// ---------------------------------------------------------------------------------------------
// As a job, and never at the same time as an install
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_started_job_uninstalls_and_is_labelled() {
    let bench = Bench::new();
    bench.install("node", "24.22.2");
    let uninstaller = Uninstaller::new(bench.home.clone());

    let started = uninstaller
        .start(&bench.jobs, "node", "24.22.2", plain())
        .unwrap();
    let info = bench.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Succeeded);
    assert_eq!(info.key, "node@24.22.2");
    assert_eq!(info.label, "Uninstall node 24.22.2");
    assert!(bench.installed().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_an_uninstall_while_the_same_version_is_installing_is_refused() {
    let bench = Bench::new();
    // A running install job of that runtime and version (it just waits to be cancelled).
    let running = bench
        .jobs
        .start("php@8.3.14", "Install PHP 8.3.14", |ctx| async move {
            ctx.cancel_token().cancelled().await;
            Ok(())
        });
    let uninstaller = Uninstaller::new(bench.home.clone());

    let err = uninstaller
        .start(&bench.jobs, "php", "8.3.14", plain())
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::Busy);
    assert_eq!(err.details.unwrap()["job"], running.id);
    bench.jobs.cancel(&running.id).unwrap();
    bench.jobs.wait(&running.id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_an_install_while_the_same_version_is_uninstalling_is_refused() {
    let bench = Bench::new();
    bench.install("php", "8.3.14");
    let (release, wait) = mpsc::channel::<()>();
    let uninstaller = Uninstaller::new(bench.home.clone()).with_usage({
        let wait = Mutex::new(wait);
        Arc::new(move |_, _| {
            wait.lock().unwrap().recv().ok();
            Ok(RuntimeUsage::default())
        })
    });
    let running = uninstaller
        .start(&bench.jobs, "php", "8.3.14", plain())
        .unwrap();
    while bench
        .jobs
        .get(&running.id)
        .is_none_or(|j| j.phase != "preparing")
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let installer = Installer::new(
        bench.home.clone(),
        Downloader::new(DownloadOptions::loopback_test()).unwrap(),
    );
    let registry = Arc::new(Registry::parse(r#"{ "schema": 1 }"#).unwrap());
    let err = installer
        .start(&bench.jobs, registry, "php", "8.3.14")
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::Busy);
    release.send(()).unwrap();
    bench.jobs.wait(&running.id).await.unwrap();
    assert!(
        bench.installed().is_empty(),
        "the uninstall was not disturbed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_uninstall_request_joins_the_running_one() {
    let bench = Bench::new();
    bench.install("php", "8.3.14");
    let (release, wait) = mpsc::channel::<()>();
    let uninstaller = Uninstaller::new(bench.home.clone()).with_usage({
        let wait = Mutex::new(wait);
        Arc::new(move |_, _| {
            wait.lock().unwrap().recv().ok();
            Ok(RuntimeUsage::default())
        })
    });

    let first = uninstaller
        .start(&bench.jobs, "php", "8.3.14", plain())
        .unwrap();
    let second = uninstaller
        .start(&bench.jobs, "php", "8.3.14", plain())
        .unwrap();

    assert!(!first.already_running && second.already_running);
    assert_eq!(first.id, second.id);
    release.send(()).unwrap();
    bench.jobs.wait(&first.id).await.unwrap();
}

/// Keeps the helper used for building paths honest on every platform.
#[test]
fn the_runtime_paths_used_here_match_the_layout() {
    let home = LunaHome::at(TempDir::new().unwrap().keep().join("lüna home")).unwrap();
    let dir: &Path = &home.runtime_dir("php", "8.3.14").unwrap();
    assert!(dir.ends_with(Path::new("runtimes").join("php").join("8.3.14")));
}

#[test]
fn the_adapters_name_the_folders_that_belong_to_a_version() {
    use luna_core::runtimes::adapter_for;
    let home = LunaHome::at(TempDir::new().unwrap().keep().join("lüna home")).unwrap();
    let data =
        |runtime: &str, version: &str| adapter_for(runtime).unwrap().data_dirs(&home, version);

    // One data folder per line, shared by the patch versions of it.
    assert_eq!(data("mysql", "8.4.13"), [home.data_dir().join("mysql-8.4")]);
    assert_eq!(data("mysql", "8.4.2"), data("mysql", "8.4.13"));
    assert_eq!(
        data("mariadb", "10.11.19"),
        [home.data_dir().join("mariadb-10.11")]
    );
    assert_ne!(
        data("mariadb", "10.11.19"),
        data("mariadb", "10.1.48"),
        "10.11 is not 10.1"
    );
    assert!(data("mysql", "not-a-version").is_empty());

    // Only databases have data; only PHP has config.
    for runtime in ["php", "node", "composer", "caddy"] {
        assert!(data(runtime, "1.2.3").is_empty(), "{runtime}");
    }
    let config = |runtime: &str| adapter_for(runtime).unwrap().config_dirs(&home, "8.3.14");
    assert_eq!(
        config("php"),
        [home.config_dir().join("php").join("8.3.14")]
    );
    assert!(
        adapter_for("php")
            .unwrap()
            .config_dirs(&home, "../escape")
            .is_empty()
    );
    for runtime in ["mysql", "mariadb", "node", "composer", "caddy"] {
        assert!(config(runtime).is_empty(), "{runtime}");
    }
}
