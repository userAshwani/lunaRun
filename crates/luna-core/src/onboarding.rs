//! The first run: where lunaRun keeps its data, what the computer has, and the "Laravel starter pack" (FR-AP-06,
//! FR-AP-03; D-044).
//!
//! - **First run** is: the wizard was not finished or skipped (`config/onboarding.json`), nothing is installed and there
//!   is no project. An install that already has something never sees the wizard.
//! - **The data folder** (LUNA_HOME) can be chosen **on the first run only** (moving one that holds runtimes and databases
//!   is Phase 3). The choice is written to a small pointer file in the OS's configuration folder ([`pointer_file`]),
//!   which [`crate::paths::LunaHome::resolve`] reads after the `LUNA_HOME` environment variable and before the default.
//!   A folder is accepted when it does not exist, is empty, or already is a lunaRun data folder; anything else could mix
//!   lunaRun's files with the user's own.
//! - **The starter pack** is four ordinary installs: the newest supported PHP, the newest Composer, the newest Node.js
//!   LTS and MySQL of the line project detection suggests (8.4), each the newest patch available for this computer.

use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};
use crate::paths::LunaHome;
use crate::projects::DetectionData;
use crate::registry::{Channel, RuntimeCatalog, RuntimeVersion};
use crate::runtimes::InstalledRuntime;
use crate::state::{self, Persisted};

/// The MySQL line of the pack: the one project detection suggests for `DB_CONNECTION=mysql`, so a new Laravel project
/// finds it installed.
pub const STARTER_MYSQL_LINE: &str = crate::projects::DEFAULT_MYSQL_LINE;

/// Free space the pack needs on top of its downloads (unpacking, the first database).
const SPACE_MARGIN: u64 = 1024 * 1024 * 1024;

// ---------------------------------------------------------------------------------------------
// First run
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardingFile {
    /// The wizard was finished or skipped.
    completed: bool,
}

impl Persisted for OnboardingFile {
    const SCHEMA_VERSION: u32 = 1;
}

fn onboarding_path(home: &LunaHome) -> PathBuf {
    home.config_dir().join("onboarding.json")
}

/// Is this the first run? The wizard is shown only then.
///
/// # Errors
/// State-file errors.
pub fn is_first_run(home: &LunaHome, installed: usize, projects: usize) -> Result<bool> {
    if installed > 0 || projects > 0 {
        return Ok(false);
    }
    let path = onboarding_path(home);
    if !path.exists() {
        return Ok(true);
    }
    Ok(!state::load::<OnboardingFile>(&path)?.value.completed)
}

/// The wizard was finished or skipped: it is not shown again.
///
/// # Errors
/// I/O errors.
pub fn complete(home: &LunaHome) -> Result<()> {
    state::save(&onboarding_path(home), &OnboardingFile { completed: true })
}

// ---------------------------------------------------------------------------------------------
// The data folder
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PointerFile {
    home: String,
}

impl Persisted for PointerFile {
    const SCHEMA_VERSION: u32 = 1;
}

/// Where the chosen data folder is remembered: a file next to (not inside) the default data folder's parent, in the OS's
/// configuration folder (`%APPDATA%\lunaRun-home.json`, `~/Library/Application Support/lunaRun-home.json`,
/// `~/.config/lunarun-home.json`). `None` when the system has no such folder.
pub fn pointer_file() -> Option<PathBuf> {
    let name = if cfg!(target_os = "linux") {
        "lunarun-home.json"
    } else {
        "lunaRun-home.json"
    };
    BaseDirs::new().map(|dirs| dirs.config_dir().join(name))
}

/// The folder a pointer file names, when it is there and names a usable folder. A damaged or odd one is ignored (the
/// default is used), never fatal.
pub fn read_pointer(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let home = PathBuf::from(value.get("home")?.as_str()?);
    LunaHome::at(&home).ok().map(|_| home)
}

/// What was noticed about a chosen folder that does not stop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HomeWarning {
    /// It is inside a folder a cloud service synchronises (OneDrive, Dropbox, iCloud, Google Drive): databases and
    /// thousands of runtime files there are slow and can be damaged by a sync.
    Synced,
    /// It already is a lunaRun data folder: what is in it is used.
    Existing,
}

/// A data folder the user may choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomeChoice {
    pub path: String,
    pub warnings: Vec<HomeWarning>,
}

fn refuse_home(path: &Path, reason: &str, message: String) -> LunaError {
    LunaError::new(ErrorCode::InvalidInput, message).with_details(json!({
        "field": "home",
        "reason": reason,
        "path": path.display().to_string(),
    }))
}

fn looks_synced(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_lowercase();
        name.starts_with("onedrive")
            || name == "dropbox"
            || name.contains("icloud")
            || name.contains("mobile documents")
            || name == "google drive"
            || name.starts_with("googledrive")
    })
}

/// Can `path` be the data folder? It must be a valid LUNA_HOME ([`LunaHome::at`]: absolute, not a drive root, not the
/// user's home) and **not exist, be empty, or already be a lunaRun data folder**; a folder with other things in it is
/// refused, so lunaRun's files never mix with the user's. Reads only.
///
/// # Errors
/// `INVALID_PATH` from [`LunaHome::at`]; `INVALID_INPUT` with `details.reason` `notEmpty` or `notFolder`.
pub fn check_home(path: &Path) -> Result<HomeChoice> {
    LunaHome::at(path)?;
    let mut warnings = Vec::new();
    if looks_synced(path) {
        warnings.push(HomeWarning::Synced);
    }
    if path.exists() {
        if !path.is_dir() {
            return Err(refuse_home(
                path,
                "notFolder",
                format!("{} is not a folder", path.display()),
            ));
        }
        let entries: Vec<String> = fs::read_dir(path)
            .map_err(|err| refuse_home(path, "notFolder", format!("{}: {err}", path.display())))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        let is_luna = path.join("config").is_dir() && path.join("runtimes").is_dir();
        if is_luna {
            warnings.push(HomeWarning::Existing);
        } else if !entries.is_empty() {
            return Err(refuse_home(
                path,
                "notEmpty",
                format!(
                    "{} already has other files in it: choose an empty folder or a new one",
                    path.display()
                ),
            ));
        }
    }
    Ok(HomeChoice {
        path: path.display().to_string(),
        warnings,
    })
}

/// Free space where `path` is (or would be: its nearest existing parent), when the system says.
pub fn free_space(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    fs4::available_space(existing).ok()
}

/// Remember `path` as the data folder for the next start: create it, prove it can be written, then write the pointer
/// file `pointer` atomically. The running app keeps its current folder until it restarts.
///
/// # Errors
/// As [`check_home`], `PERMISSION_DENIED` or I/O errors when the folder cannot be made or written.
pub fn choose_home(path: &Path, pointer: &Path) -> Result<HomeChoice> {
    let choice = check_home(path)?;
    let home = LunaHome::at(path)?;
    home.ensure_layout()?;
    // Writing a file is the only proof the folder can be used; the probe is removed through the one deleting door.
    let probe = home.config_dir().join(".write-test");
    state::write_atomic(&probe, b"lunaRun")?;
    home.safe_remove(&probe)?;
    state::save(
        pointer,
        &PointerFile {
            home: path.display().to_string(),
        },
    )?;
    Ok(choice)
}

// ---------------------------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------------------------

/// The Visual C++ runtime the Windows builds of PHP need (FR-RT-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VcRedistState {
    /// macOS and Linux: nothing to check.
    NotNeeded,
    Ok,
    Missing,
    Outdated,
    /// The registry could not be read: nothing is known (an install then fails with a clear message if it is missing).
    Unknown,
}

/// What the wizard checks before it offers anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemChecks {
    pub vc_redist: VcRedistState,
    /// Free space where the data folder is, when the system says.
    #[ts(type = "number | null")]
    pub free_bytes: Option<u64>,
    /// The free space is enough for the whole pack (with a margin).
    pub enough_space: bool,
}

/// Put the checks together. `vc_redist` is the Windows check's answer (`Ok(None)`: not installed), ignored elsewhere.
pub fn checks(
    os: Os,
    vc_redist: Result<Option<crate::runtimes::prerequisites::VcRedist>>,
    free_bytes: Option<u64>,
    pack_bytes: u64,
) -> SystemChecks {
    let vc_redist = if os != Os::Windows {
        VcRedistState::NotNeeded
    } else {
        match vc_redist {
            Ok(Some(found)) if found.installed && found.is_recent_enough() => VcRedistState::Ok,
            Ok(Some(found)) if found.installed => VcRedistState::Outdated,
            Ok(_) => VcRedistState::Missing,
            Err(_) => VcRedistState::Unknown,
        }
    };
    SystemChecks {
        vc_redist,
        free_bytes,
        // Unknown free space does not block anything (the installer checks again, D-019).
        enough_space: free_bytes.is_none_or(|free| free >= pack_bytes.saturating_add(SPACE_MARGIN)),
    }
}

// ---------------------------------------------------------------------------------------------
// The starter pack
// ---------------------------------------------------------------------------------------------

/// One runtime of the pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackItem {
    pub runtime: String,
    pub version: String,
    pub line: String,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// A version of the same line is installed already (that one, when so): nothing to install.
    pub installed_version: Option<String>,
}

/// The pack for this computer, and what of it cannot be had here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StarterPack {
    pub items: Vec<PackItem>,
    /// Runtimes of the pack with no download for this computer (MySQL on Linux on ARM, for example).
    pub unavailable: Vec<String>,
}

impl StarterPack {
    /// What the items still to install download.
    pub fn download_bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|item| item.installed_version.is_none())
            .filter_map(|item| item.size)
            .sum()
    }
}

fn major(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

/// Which versions of a runtime may be in the pack (on top of stable, supported and available).
type Fits = Box<dyn Fn(&crate::registry::AvailableVersion) -> bool>;

/// Choose the pack from what the registry offers here (`catalog`, newest first), what is installed and the Node.js LTS
/// lines (`data`).
pub fn starter_pack(
    catalog: &[RuntimeCatalog],
    installed: &[InstalledRuntime],
    data: &DetectionData,
) -> StarterPack {
    let lts = data.lts_majors();
    let wanted: [(&str, Fits); 4] = [
        ("php", Box::new(|_| true)),
        ("composer", Box::new(|_| true)),
        (
            "node",
            Box::new(move |v| major(&v.version).is_some_and(|m| lts.contains(&m))),
        ),
        (
            "mysql",
            Box::new(|v| {
                RuntimeVersion::parse(&v.version).is_some_and(|r| r.is_on_line(STARTER_MYSQL_LINE))
            }),
        ),
    ];
    let mut pack = StarterPack {
        items: Vec::new(),
        unavailable: Vec::new(),
    };
    for (runtime, fits) in wanted {
        let chosen = catalog.iter().find(|c| c.id == runtime).and_then(|c| {
            c.versions
                .iter()
                .find(|v| v.available && v.channel == Channel::Stable && !v.eol && fits(v))
        });
        let Some(version) = chosen else {
            pack.unavailable.push(runtime.to_owned());
            continue;
        };
        let installed_version = installed
            .iter()
            .filter(|r| {
                r.runtime == runtime
                    && RuntimeVersion::parse(&r.version)
                        .is_some_and(|v| v.is_on_line(&version.line))
            })
            .max_by_key(|r| RuntimeVersion::parse(&r.version))
            .map(|r| r.version.clone());
        pack.items.push(PackItem {
            runtime: runtime.to_owned(),
            version: version.version.clone(),
            line: version.line.clone(),
            size: version.size,
            installed_version,
        });
    }
    pack
}

// ---------------------------------------------------------------------------------------------
// What the wizard shows
// ---------------------------------------------------------------------------------------------

/// Where the data folder came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HomeSource {
    /// The `LUNA_HOME` environment variable: it cannot be changed from the app.
    Env,
    /// Chosen in the wizard.
    Chosen,
    Default,
}

/// Everything the first-run wizard needs, in one answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OnboardingView {
    /// It is the first run: show the wizard.
    pub show: bool,
    pub home: String,
    pub home_source: HomeSource,
    /// Another data folder can be chosen now (first run, and not set by the environment).
    pub can_choose_home: bool,
    pub checks: SystemChecks,
    pub pack: StarterPack,
}

/// Put the wizard's answer together. The system is asked through `vc_redist` and `free_space`, so this is tested anywhere.
///
/// # Errors
/// State-file errors.
#[allow(clippy::too_many_arguments)]
pub fn overview(
    home: &LunaHome,
    os: Os,
    catalog: &[RuntimeCatalog],
    installed: &[InstalledRuntime],
    projects: usize,
    data: &DetectionData,
    vc_redist: impl FnOnce() -> Result<Option<crate::runtimes::prerequisites::VcRedist>>,
    free_space: impl FnOnce(&Path) -> Option<u64>,
) -> Result<OnboardingView> {
    let show = is_first_run(home, installed.len(), projects)?;
    let pack = starter_pack(catalog, installed, data);
    let vc = if os == Os::Windows {
        vc_redist()
    } else {
        Ok(None)
    };
    let checks = checks(os, vc, free_space(home.root()), pack.download_bytes());
    let home_source = if home.from_env() {
        HomeSource::Env
    } else if home.is_chosen() {
        HomeSource::Chosen
    } else {
        HomeSource::Default
    };
    Ok(OnboardingView {
        show,
        home: home.root().display().to_string(),
        home_source,
        can_choose_home: show && home_source != HomeSource::Env,
        checks,
        pack,
    })
}

/// Choose the data folder, when that is still possible: on the first run and when the environment does not set it.
///
/// # Errors
/// `INVALID_INPUT` (`details.reason` `notFirstRun` or `fromEnv`), and what [`choose_home`] answers.
pub fn choose_home_now(
    home: &LunaHome,
    installed: usize,
    projects: usize,
    path: &Path,
    pointer: &Path,
) -> Result<HomeChoice> {
    if home.from_env() {
        return Err(refuse_home(
            path,
            "fromEnv",
            "the data folder is set by the LUNA_HOME environment variable".to_owned(),
        ));
    }
    if !is_first_run(home, installed, projects)? {
        return Err(refuse_home(
            path,
            "notFirstRun",
            "the data folder can only be chosen before anything is installed".to_owned(),
        ));
    }
    choose_home(path, pointer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, LunaHome) {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        home.ensure_layout().unwrap();
        (tmp, home)
    }

    #[test]
    fn the_first_run_is_until_the_wizard_is_done_and_never_once_something_exists() {
        let (_tmp, home) = home();
        assert!(is_first_run(&home, 0, 0).unwrap());
        assert!(!is_first_run(&home, 1, 0).unwrap(), "something installed");
        assert!(!is_first_run(&home, 0, 2).unwrap(), "a project");
        complete(&home).unwrap();
        assert!(!is_first_run(&home, 0, 0).unwrap());
    }

    #[test]
    fn a_new_or_empty_or_lunarun_folder_can_be_chosen_and_one_with_other_files_cannot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let new = tmp.path().join("Données lunaRun");
        assert_eq!(check_home(&new).unwrap().warnings, []);

        let empty = tmp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert_eq!(check_home(&empty).unwrap().warnings, []);

        let busy = tmp.path().join("my work");
        fs::create_dir(&busy).unwrap();
        fs::write(busy.join("notes.txt"), "mine").unwrap();
        let err = check_home(&busy).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(err.details.unwrap()["reason"], "notEmpty");

        let file = tmp.path().join("a file");
        fs::write(&file, "x").unwrap();
        assert_eq!(
            check_home(&file).unwrap_err().details.unwrap()["reason"],
            "notFolder"
        );

        let existing = tmp.path().join("old luna");
        LunaHome::at(&existing).unwrap().ensure_layout().unwrap();
        assert_eq!(
            check_home(&existing).unwrap().warnings,
            [HomeWarning::Existing]
        );

        assert_eq!(
            check_home(Path::new("relative/folder")).unwrap_err().code,
            ErrorCode::InvalidPath
        );
    }

    #[test]
    fn a_folder_a_cloud_service_synchronises_is_allowed_with_a_warning() {
        let tmp = tempfile::TempDir::new().unwrap();
        for synced in ["OneDrive - Contoso", "OneDrive", "Dropbox", "Google Drive"] {
            let path = tmp.path().join(synced).join("lunaRun");
            assert_eq!(
                check_home(&path).unwrap().warnings,
                [HomeWarning::Synced],
                "{synced}"
            );
        }
        assert_eq!(
            check_home(&tmp.path().join("Projects").join("lunaRun"))
                .unwrap()
                .warnings,
            []
        );
    }

    #[test]
    fn choosing_a_folder_makes_it_proves_it_can_be_written_and_remembers_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let chosen = tmp.path().join("Daten lünaRun");
        let pointer = tmp.path().join("config").join("lunaRun-home.json");

        let choice = choose_home(&chosen, &pointer).unwrap();

        assert_eq!(choice.path, chosen.display().to_string());
        assert!(chosen.join("runtimes").is_dir() && chosen.join("config").is_dir());
        assert!(!chosen.join("config").join(".write-test").exists());
        assert_eq!(read_pointer(&pointer), Some(chosen));
    }

    #[test]
    fn a_damaged_or_odd_pointer_is_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pointer = tmp.path().join("p.json");
        assert_eq!(read_pointer(&pointer), None, "missing");
        fs::write(&pointer, "{not json").unwrap();
        assert_eq!(read_pointer(&pointer), None);
        fs::write(&pointer, r#"{"schema":1,"home":"relative/path"}"#).unwrap();
        assert_eq!(read_pointer(&pointer), None);
        fs::write(&pointer, r#"{"schema":1}"#).unwrap();
        assert_eq!(read_pointer(&pointer), None);
    }

    #[test]
    fn the_checks_say_what_matters_on_each_system() {
        use crate::runtimes::prerequisites::VcRedist;
        let vc = |major, minor, installed| VcRedist {
            installed,
            major,
            minor,
            build: 0,
            version: None,
        };
        let gb = 1024 * 1024 * 1024;
        assert_eq!(
            checks(Os::Windows, Ok(Some(vc(14, 51, true))), Some(10 * gb), gb).vc_redist,
            VcRedistState::Ok
        );
        assert_eq!(
            checks(Os::Windows, Ok(Some(vc(14, 10, true))), None, 0).vc_redist,
            VcRedistState::Outdated
        );
        assert_eq!(
            checks(Os::Windows, Ok(None), None, 0).vc_redist,
            VcRedistState::Missing
        );
        assert_eq!(
            checks(
                Os::Windows,
                Err(LunaError::internal("reg is not there")),
                None,
                0
            )
            .vc_redist,
            VcRedistState::Unknown
        );
        assert_eq!(
            checks(Os::Macos, Ok(None), None, 0).vc_redist,
            VcRedistState::NotNeeded
        );
        // Space: the pack plus a gigabyte; unknown space does not block.
        assert!(checks(Os::Linux, Ok(None), Some(2 * gb), gb / 2).enough_space);
        assert!(!checks(Os::Linux, Ok(None), Some(gb), gb / 2).enough_space);
        assert!(checks(Os::Linux, Ok(None), None, 50 * gb).enough_space);
    }

    #[test]
    fn the_data_folder_can_be_chosen_only_on_the_first_run() {
        let (tmp, home) = home();
        let pointer = tmp.path().join("pointer.json");
        let target = tmp.path().join("elsewhere");
        let err = choose_home_now(&home, 1, 0, &target, &pointer).unwrap_err();
        assert_eq!(err.details.unwrap()["reason"], "notFirstRun");
        assert!(!pointer.exists() && !target.exists(), "nothing written");
        choose_home_now(&home, 0, 0, &target, &pointer).unwrap();
        assert_eq!(read_pointer(&pointer), Some(target));
    }
}
