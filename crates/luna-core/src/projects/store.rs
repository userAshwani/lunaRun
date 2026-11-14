//! The list of projects the user added, kept in `config/projects.json` (docs/ARCHITECTURE.md section 5; D-036).
//!
//! What is stored is what only the user knows: which folder, what it is called, which folder is served, and the
//! PHP or Node version they chose by hand. **What a project needs is never stored**: [`ProjectStore::inspect`] reads the
//! folder again each time ([`detect`]) and resolves it ([`resolve`]), so the answer follows the files. The one thing
//! kept from a scan is the hash of its inputs, so a later session can tell the project changed.
//!
//! Nothing here touches a project folder except to read it. **Removing a project forgets it; it never deletes a file**
//! (FR-PJ, golden rule 2): the store has no code path that deletes anything but its own entry.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::{
    Binding, BindingMode, DetectionData, DetectionReport, Framework, Resolution, ResolvePrefs,
    WarningCode, detect, resolve,
};
use crate::diagnostics::Os;
use crate::download::to_hex;
use crate::error::{ErrorCode, LunaError, Result};
use crate::paths::LunaHome;
use crate::registry::RuntimeCatalog;
use crate::runtimes::InstalledRuntime;
use crate::state::{self, Persisted};

/// A name longer than this is not a name.
const MAX_NAME_CHARS: usize = 100;

/// The versions the user chose by hand for one project. `None` is "follow what the project says" (automatic).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bindings {
    pub php: Option<Binding>,
    pub node: Option<Binding>,
}

/// When a project was last read, and what its input files hashed to then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LastDetection {
    /// Seconds since 1970-01-01 UTC.
    #[ts(type = "number")]
    pub at: u64,
    pub hash: String,
}

/// One project of the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    /// Random, never reused; what every command takes.
    pub id: String,
    pub name: String,
    /// Absolute, as the user gave it (no canonical form: on Windows that would add a `\\?\` prefix to what they see).
    pub path: String,
    /// The folder that is served, relative to `path` with `/`; `.` is the folder itself.
    pub doc_root: String,
    /// The user chose `doc_root`; a re-scan then leaves it alone instead of following detection.
    #[serde(default)]
    pub doc_root_custom: bool,
    /// From the last scan; kept so the list can show it without reading every folder.
    #[serde(default)]
    pub framework: Option<Framework>,
    #[serde(default)]
    pub bindings: Bindings,
    #[serde(default)]
    pub last_detection: Option<LastDetection>,
    /// Seconds since 1970-01-01 UTC.
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
    pub updated_at: u64,
}

/// A manual choice for a binding: a version, or back to automatic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum BindingChoice {
    Auto,
    Use { binding: Binding },
}

/// A manual choice for the document root: a folder of the project, or back to what detection finds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum DocRootChoice {
    Auto,
    Use { path: String },
}

/// What `project_update` changes. A field that is absent is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ProjectPatch {
    #[ts(optional)]
    pub name: Option<String>,
    #[ts(optional)]
    pub doc_root: Option<DocRootChoice>,
    #[ts(optional)]
    pub php: Option<BindingChoice>,
    #[ts(optional)]
    pub node: Option<BindingChoice>,
}

/// A project with what its folder says today and what to run it with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectDetection {
    pub project: Project,
    pub report: DetectionReport,
    pub resolution: Resolution,
}

/// What a project is resolved against: what is installed, what could be, and which system this is.
#[derive(Clone, Copy)]
pub struct Surroundings<'a> {
    pub installed: &'a [InstalledRuntime],
    pub available: &'a [RuntimeCatalog],
    pub os: Os,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProjectsFile {
    #[serde(default)]
    projects: Vec<Project>,
}

impl Persisted for ProjectsFile {
    const SCHEMA_VERSION: u32 = 1;
}

/// The projects, on disk and in memory. Cheap to clone: clones share one list.
#[derive(Clone)]
pub struct ProjectStore {
    file: PathBuf,
    state: Arc<Mutex<ProjectsFile>>,
}

impl ProjectStore {
    /// Open `config/projects.json` under `home`. A damaged file is moved aside and the list starts empty
    /// ([`state::load`]); one written by a newer lunaRun is an error and is left untouched.
    pub fn open(home: &LunaHome) -> Result<Self> {
        let file = home.config_dir().join("projects.json");
        let loaded = state::load::<ProjectsFile>(&file)?;
        Ok(Self {
            file,
            state: Arc::new(Mutex::new(loaded.value)),
        })
    }

    /// Every project, in the order they were added.
    pub fn list(&self) -> Vec<Project> {
        self.lock().projects.clone()
    }

    pub fn get(&self, id: &str) -> Result<Project> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| not_found(id))
    }

    /// Add the folder `path` as a project, read it, and say what it needs. The folder must exist and be absolute;
    /// adding the same folder twice is `PROJECT_EXISTS`. Nothing is written into the folder.
    pub fn add(
        &self,
        path: &Path,
        data: &DetectionData,
        around: &Surroundings<'_>,
    ) -> Result<ProjectDetection> {
        // `components` drops a trailing separator and `.` parts without changing what the user typed otherwise.
        let given: PathBuf = path.components().collect();
        let shown = given.display().to_string();
        let at = |message: String| {
            LunaError::new(ErrorCode::InvalidPath, message).with_details(json!({ "path": shown }))
        };
        if !given.is_absolute() {
            return Err(at(format!("{shown} is not an absolute path")));
        }
        if !given.is_dir() {
            return Err(at(format!("{shown} is not a folder that exists")));
        }
        let text = given
            .to_str()
            .ok_or_else(|| at(format!("{shown} is not a path lunaRun can store")))?
            .to_owned();

        let report = detect(&given, data);
        let stamp = now();
        let project = self.change(|file| {
            if let Some(existing) = file
                .projects
                .iter()
                .find(|p| same_folder(Path::new(&p.path), &given))
            {
                return Err(LunaError::new(
                    ErrorCode::ProjectExists,
                    format!("{shown} is already the project {}", existing.name),
                )
                .with_details(json!({ "id": existing.id, "name": existing.name })));
            }
            let project = Project {
                id: new_id(&text),
                name: folder_name(&given),
                path: text,
                doc_root: report.doc_root.path.clone(),
                doc_root_custom: false,
                framework: report.framework.clone(),
                bindings: Bindings::default(),
                last_detection: Some(LastDetection {
                    at: stamp,
                    hash: report.inputs_hash.clone(),
                }),
                created_at: stamp,
                updated_at: stamp,
            };
            file.projects.push(project.clone());
            Ok(project)
        })?;
        let resolution = resolve_for(&project, &report, data, around);
        Ok(ProjectDetection {
            project,
            report,
            resolution,
        })
    }

    /// Read the project's folder again and resolve it. The framework, the hash of the inputs and (unless the user
    /// chose one) the document root of the stored project follow what the folder says now. A folder that has gone
    /// missing is reported as a warning and the stored project is left as it was.
    pub fn inspect(
        &self,
        id: &str,
        data: &DetectionData,
        around: &Surroundings<'_>,
    ) -> Result<ProjectDetection> {
        let project = self.get(id)?;
        let report = detect(Path::new(&project.path), data);
        let gone = report
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::ProjectFolderMissing);
        let project = if gone {
            project
        } else {
            let stamp = now();
            self.change(|file| {
                let entry = find_mut(file, id)?;
                entry.framework = report.framework.clone();
                if !entry.doc_root_custom {
                    entry.doc_root = report.doc_root.path.clone();
                }
                entry.last_detection = Some(LastDetection {
                    at: stamp,
                    hash: report.inputs_hash.clone(),
                });
                Ok(entry.clone())
            })?
        };
        let resolution = resolve_for(&project, &report, data, around);
        Ok(ProjectDetection {
            project,
            report,
            resolution,
        })
    }

    /// Change what `patch` names. Every part is checked before anything is saved, so a bad part changes nothing.
    pub fn update(&self, id: &str, patch: &ProjectPatch, data: &DetectionData) -> Result<Project> {
        let project = self.get(id)?;
        let name = patch.name.as_deref().map(clean_name).transpose()?;
        let doc_root = match &patch.doc_root {
            None => None,
            Some(DocRootChoice::Auto) => {
                let found = detect(Path::new(&project.path), data).doc_root.path;
                Some((found, false))
            }
            Some(DocRootChoice::Use { path }) => {
                Some((normalise_doc_root(Path::new(&project.path), path)?, true))
            }
        };
        let php = patch_binding("php", patch.php.as_ref())?;
        let node = patch_binding("node", patch.node.as_ref())?;
        if name.is_none() && doc_root.is_none() && php.is_none() && node.is_none() {
            return Ok(project);
        }
        let stamp = now();
        self.change(|file| {
            let entry = find_mut(file, id)?;
            if let Some(name) = name {
                entry.name = name;
            }
            if let Some((path, custom)) = doc_root {
                entry.doc_root = path;
                entry.doc_root_custom = custom;
            }
            if let Some(php) = php {
                entry.bindings.php = php;
            }
            if let Some(node) = node {
                entry.bindings.node = node;
            }
            entry.updated_at = stamp;
            Ok(entry.clone())
        })
    }

    /// Forget a project. **The folder and everything in it stay exactly as they are.**
    pub fn remove(&self, id: &str) -> Result<()> {
        self.change(|file| {
            let before = file.projects.len();
            file.projects.retain(|p| p.id != id);
            if file.projects.len() == before {
                return Err(not_found(id));
            }
            Ok(())
        })
    }

    /// The names of the projects whose manual choice is this installed version: a line binding names the version's
    /// line (`8.3` for PHP 8.3.14, `22` for Node 22.11.0), an exact one the version itself. What a project follows
    /// automatically is not a binding, so it is not listed (D-036).
    pub fn bound_to(&self, runtime: &str, version: &str) -> Vec<String> {
        let chosen = |bindings: &Bindings| match runtime {
            "php" => bindings.php.clone(),
            "node" => bindings.node.clone(),
            _ => None,
        };
        let line = line_of(runtime, version);
        self.lock()
            .projects
            .iter()
            .filter(|p| {
                chosen(&p.bindings).is_some_and(|b| match b.mode {
                    BindingMode::Exact => b.value == version,
                    BindingMode::Line => Some(b.value.as_str()) == line.as_deref(),
                })
            })
            .map(|p| p.name.clone())
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ProjectsFile> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Apply `edit` to a copy, save it, and only then make it the list: a failed save leaves memory as it was.
    fn change<R>(&self, edit: impl FnOnce(&mut ProjectsFile) -> Result<R>) -> Result<R> {
        let mut guard = self.lock();
        let mut next = guard.clone();
        let out = edit(&mut next)?;
        state::save(&self.file, &next)?;
        *guard = next;
        Ok(out)
    }
}

fn resolve_for(
    project: &Project,
    report: &DetectionReport,
    data: &DetectionData,
    around: &Surroundings<'_>,
) -> Resolution {
    let mut prefs = ResolvePrefs::new(around.os);
    prefs.php_override = project.bindings.php.as_ref().map(|b| b.value.clone());
    prefs.node_override = project.bindings.node.as_ref().map(|b| b.value.clone());
    resolve(report, around.installed, around.available, data, &prefs)
}

fn not_found(id: &str) -> LunaError {
    LunaError::new(ErrorCode::NotFound, format!("no project with the id {id}"))
        .with_details(json!({ "id": id }))
}

fn find_mut<'a>(file: &'a mut ProjectsFile, id: &str) -> Result<&'a mut Project> {
    file.projects
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| not_found(id))
}

fn invalid(field: &str, message: impl Into<String>) -> LunaError {
    LunaError::new(ErrorCode::InvalidInput, message).with_details(json!({ "field": field }))
}

fn clean_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(invalid("name", "the project name is empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(
            "name",
            format!("the project name is longer than {MAX_NAME_CHARS} characters"),
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid("name", "the project name has a control character"));
    }
    Ok(name.to_owned())
}

/// The last part of the folder, or the whole path for a root such as `C:\`.
fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A document root as the page may send it, as `a/b` relative to the project: no way out of the project (`..`, an
/// absolute path, a drive), and the folder has to exist.
fn normalise_doc_root(project: &Path, raw: &str) -> Result<String> {
    let text = raw.trim().replace('\\', "/");
    if text.starts_with('/') || text.contains(':') || text.contains('\0') {
        return Err(invalid(
            "docRoot",
            format!("{raw} is not a folder inside the project"),
        ));
    }
    let parts: Vec<&str> = text
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.contains(&"..") {
        return Err(invalid(
            "docRoot",
            format!("{raw} leaves the project folder"),
        ));
    }
    let relative = if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    };
    let dir = if relative == "." {
        project.to_path_buf()
    } else {
        project.join(&relative)
    };
    if !dir.is_dir() {
        return Err(invalid(
            "docRoot",
            format!("the project has no folder {relative}"),
        ));
    }
    Ok(relative)
}

/// `Some(new value)` when the patch changes the binding, `None` when it leaves it.
fn patch_binding(runtime: &str, choice: Option<&BindingChoice>) -> Result<Option<Option<Binding>>> {
    match choice {
        None => Ok(None),
        Some(BindingChoice::Auto) => Ok(Some(None)),
        Some(BindingChoice::Use { binding }) => {
            check_binding(runtime, binding)?;
            Ok(Some(Some(binding.clone())))
        }
    }
}

/// A line is `8.3` (PHP) or `22` (Node), an exact version `8.3.14`: digits and dots, nothing else. A pre-release or a
/// range is not something a project is bound to.
fn check_binding(runtime: &str, binding: &Binding) -> Result<()> {
    let parts: Vec<&str> = binding.value.split('.').collect();
    let numeric = parts
        .iter()
        .all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()));
    let line_parts = if runtime == "php" { 2 } else { 1 };
    let wanted = match binding.mode {
        BindingMode::Line => line_parts,
        BindingMode::Exact => 3,
    };
    if numeric && parts.len() == wanted {
        return Ok(());
    }
    let what = match binding.mode {
        BindingMode::Line if runtime == "php" => "a PHP line such as 8.3",
        BindingMode::Line => "a Node line such as 22",
        BindingMode::Exact => "an exact version such as 8.3.14",
    };
    Err(invalid(runtime, format!("{} is not {what}", binding.value)))
}

/// `8.3` for PHP 8.3.14, `22` for Node 22.11.0; `None` for a runtime with no line binding yet.
fn line_of(runtime: &str, version: &str) -> Option<String> {
    let mut parts = version.split('.');
    match runtime {
        "php" => Some(format!("{}.{}", parts.next()?, parts.next()?)),
        "node" => parts.next().map(str::to_owned),
        _ => None,
    }
}

/// The same folder, whatever the spelling (case on Windows, a link): by what the OS says it is when it can, else as given.
fn same_folder(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A random-looking, unique id: a hash of the time, a counter, the process and the path. Ids only have to differ.
fn new_id(path: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(path.as_bytes());
    to_hex(&hasher.finalize())[..32].to_owned()
}
