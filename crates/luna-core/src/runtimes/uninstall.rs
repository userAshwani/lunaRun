//! Uninstalling a runtime version (FR-RT-05, FR-RT-06, docs/RUNTIMES.md section 4 "Uninstall").
//!
//! ```text
//! installed? -> not in use? -> projects confirmed? -> rename runtimes/<rt>/<ver> to .trash-<job>
//!   -> remove the version's config -> delete the trash folder -> database data: keep, or delete if asked
//! ```
//!
//! - **The rename is the point of no return and the first thing that touches the disk.** After it the version
//!   is no longer listed as installed, so a deletion that dies halfway (a crash, a locked file) leaves a
//!   hidden `.trash-*` folder that [`cleanup_incomplete`](super::cleanup_incomplete) removes at the next start,
//!   never a half-deleted runtime that still looks installed. On Windows a program running from the folder
//!   makes the rename fail, which is reported as `RUNTIME_IN_USE` with nothing changed. Because the rename
//!   cannot be undone, an uninstall that has started always finishes: it ignores cancel after that point.
//! - **Database data is kept** unless the user asked to delete it, and kept even then while another installed
//!   version still uses the same data folder (patch versions of one line share it).
//! - **Services and projects do not exist yet** (M1.4, M1.7), so "is a service running" and "which projects
//!   are bound" are [`UsageChecker`] hooks. By default nothing is in use.
//! - Every deletion goes through [`LunaHome::safe_remove`], so nothing outside `LUNA_HOME` can be touched and a
//!   symlink inside a runtime folder is unlinked, never followed.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use super::adapter::RuntimeAdapter;
use super::adapters::adapter_for;
use super::install::{INSTALL_LABEL, refuse_if_other_operation};
use super::installed::{InstalledRuntime, list_installed};
use crate::error::{ErrorCode, LunaError, Result};
use crate::jobs::{JobContext, JobManager, StartedJob};
use crate::paths::LunaHome;

/// The start of an uninstall job's label (an install job's label starts with `Install `).
const UNINSTALL_LABEL: &str = "Uninstall ";

/// What currently depends on a runtime version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeUsage {
    /// Names of the running services started from it. Any makes the uninstall fail with `RUNTIME_IN_USE`.
    pub running_services: Vec<String>,
    /// Names of the projects bound to it. They are listed so the user can confirm.
    pub bound_projects: Vec<String>,
}

/// What the confirmation dialog shows before an uninstall (`runtime_usage` in the IPC list).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsageReport {
    /// Names of the running services started from this version. The uninstall is refused while any run.
    pub running_services: Vec<String>,
    /// Names of the projects bound to this version.
    pub bound_projects: Vec<String>,
    /// Existing folders that hold the user's databases for this version (empty for most runtimes).
    pub data_dirs: Vec<String>,
    /// Another installed version uses the same data folders, so asking to delete them would keep them.
    pub data_shared: bool,
}

/// Looks up what uses `(runtime, version)`. Replaced by the supervisor and the project store when they exist.
pub type UsageChecker = Arc<dyn Fn(&str, &str) -> Result<RuntimeUsage> + Send + Sync>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UninstallOptions {
    /// Also delete the database data folders of this version (FR-RT-06: only when the user explicitly asks).
    pub delete_data: bool,
    /// The user has seen the list of bound projects and wants to go on.
    pub confirm_bound_projects: bool,
}

/// What an uninstall did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallReport {
    /// The install folder that was removed.
    pub removed_dir: PathBuf,
    /// Config folders removed with the version (PHP's `config/php/<version>`).
    pub removed_config: Vec<PathBuf>,
    /// Data folders deleted because the user asked.
    pub removed_data: Vec<PathBuf>,
    /// Data folders that exist and were kept, so the UI can say where the databases are.
    pub kept_data: Vec<PathBuf>,
    /// Set when the install folder is no longer a runtime but could not be deleted completely (a locked file).
    /// It sits in a hidden `.trash-*` folder that is removed at the next start.
    pub leftover: Option<PathBuf>,
}

struct Config {
    home: LunaHome,
    usage: UsageChecker,
    adapters: fn(&str) -> Option<&'static dyn RuntimeAdapter>,
}

/// Uninstalls runtimes from a `LUNA_HOME`. Cheap to clone.
#[derive(Clone)]
pub struct Uninstaller(Arc<Config>);

impl Uninstaller {
    /// Nothing is reported as in use until the supervisor and the project store provide a checker.
    pub fn new(home: LunaHome) -> Self {
        Self(Arc::new(Config {
            home,
            usage: Arc::new(|_, _| Ok(RuntimeUsage::default())),
            adapters: adapter_for,
        }))
    }

    #[must_use]
    pub fn with_usage(self, usage: UsageChecker) -> Self {
        Self(Arc::new(Config {
            home: self.0.home.clone(),
            usage,
            adapters: self.0.adapters,
        }))
    }

    #[must_use]
    pub fn with_adapters(self, adapters: fn(&str) -> Option<&'static dyn RuntimeAdapter>) -> Self {
        Self(Arc::new(Config {
            home: self.0.home.clone(),
            usage: Arc::clone(&self.0.usage),
            adapters,
        }))
    }

    /// What uses this version right now, for the confirmation dialog (`runtime_usage` in the IPC list).
    pub fn usage(&self, runtime: &str, version: &str) -> Result<RuntimeUsage> {
        (self.0.usage)(runtime, version)
    }

    /// Everything the confirmation dialog needs about `(runtime, version)`: what uses it and where its
    /// databases are. Read-only. `NOT_FOUND` when that version is not installed.
    pub fn usage_report(&self, runtime: &str, version: &str) -> Result<UsageReport> {
        let home = &self.0.home;
        let installed = list_installed(home)?;
        let target = installed
            .iter()
            .find(|i| i.runtime == runtime && i.version == version)
            .ok_or_else(|| {
                LunaError::new(
                    ErrorCode::NotFound,
                    format!("{runtime} {version} is not installed"),
                )
            })?;
        let usage = (self.0.usage)(runtime, version)?;
        // The same plan an uninstall that deletes data would make: what it would delete is not shared,
        // what it would keep is.
        let plan = plan_data(home, (self.0.adapters)(runtime), target, &installed, true);
        let data_shared = !plan.keep.is_empty();
        let data_dirs = plan
            .delete
            .iter()
            .chain(&plan.keep)
            .map(|dir| dir.display().to_string())
            .collect();
        Ok(UsageReport {
            running_services: usage.running_services,
            bound_projects: usage.bound_projects,
            data_dirs,
            data_shared,
        })
    }

    /// Start the uninstall as a background job keyed `<runtime>@<version>`, the key an install uses too, so the
    /// two can never run at once: starting one while the other runs fails with `BUSY`.
    pub fn start(
        &self,
        jobs: &JobManager,
        runtime: &str,
        version: &str,
        options: UninstallOptions,
    ) -> Result<StartedJob> {
        let uninstaller = self.clone();
        let (runtime_id, version_text) = (runtime.to_owned(), version.to_owned());
        let started = jobs.start(
            &format!("{runtime}@{version}"),
            &format!("{UNINSTALL_LABEL}{runtime} {version}"),
            move |ctx| async move {
                uninstaller
                    .uninstall(&runtime_id, &version_text, options, &ctx)
                    .await
                    .map(|_| ())
            },
        );
        // An install of the same version is running: refuse instead of joining it.
        refuse_if_other_operation(jobs, &started, UNINSTALL_LABEL, runtime, version)?;
        debug_assert_ne!(UNINSTALL_LABEL, INSTALL_LABEL);
        Ok(started)
    }

    /// Uninstall `runtime` `version`. See the module docs.
    pub async fn uninstall(
        &self,
        runtime: &str,
        version: &str,
        options: UninstallOptions,
        ctx: &JobContext,
    ) -> Result<UninstallReport> {
        let this = self.clone();
        let (runtime, version) = (runtime.to_owned(), version.to_owned());
        let job_id = ctx.id();
        let cancel = ctx.cancel_token().clone();
        let phase_ctx = ctx.clone();
        // Moving and deleting folders is blocking work, and a large runtime (MySQL is about 1 GB) takes
        // seconds, so it never runs on an async worker thread.
        tokio::task::spawn_blocking(move || {
            this.uninstall_blocking(&runtime, &version, options, &job_id, &cancel, &phase_ctx)
        })
        .await
        .map_err(|err| LunaError::internal(format!("the uninstall task failed: {err}")))?
    }

    fn uninstall_blocking(
        &self,
        runtime: &str,
        version: &str,
        options: UninstallOptions,
        job_id: &str,
        cancel: &tokio_util::sync::CancellationToken,
        ctx: &JobContext,
    ) -> Result<UninstallReport> {
        let home = &self.0.home;
        ctx.phase("preparing", None);

        // 1. It has to be installed (a valid receipt), not just a folder with the right name.
        let installed = list_installed(home)?;
        let target = installed
            .iter()
            .find(|i| i.runtime == runtime && i.version == version)
            .ok_or_else(|| {
                LunaError::new(
                    ErrorCode::NotFound,
                    format!("{runtime} {version} is not installed"),
                )
            })?;

        // 2. Nothing may be running from it, and projects need an explicit yes.
        let usage = (self.0.usage)(runtime, version)?;
        if !usage.running_services.is_empty() {
            return Err(LunaError::new(
                ErrorCode::RuntimeInUse,
                format!(
                    "{runtime} {version} cannot be uninstalled while {} is running",
                    usage.running_services.join(", ")
                ),
            )
            .with_details(json!({ "services": usage.running_services })));
        }
        if !usage.bound_projects.is_empty() && !options.confirm_bound_projects {
            return Err(LunaError::new(
                ErrorCode::ProjectsBound,
                format!(
                    "{} project(s) use {runtime} {version}: {}",
                    usage.bound_projects.len(),
                    usage.bound_projects.join(", ")
                ),
            )
            .with_details(json!({ "projects": usage.bound_projects })));
        }
        if cancel.is_cancelled() {
            return Err(LunaError::new(
                ErrorCode::Cancelled,
                "the uninstall was cancelled",
            ));
        }

        // What belongs to this version besides its folder, decided before anything is removed.
        let adapter = (self.0.adapters)(runtime);
        let config_dirs = adapter.map_or_else(Vec::new, |a| a.config_dirs(home, version));
        let data_plan = plan_data(home, adapter, target, &installed, options.delete_data);

        // 3. The point of no return: move the folder out of the way in one step. From here on it always finishes.
        ctx.phase("removing", None);
        let trash = target
            .dir
            .parent()
            .unwrap_or(&target.dir)
            .join(format!(".trash-{job_id}"));
        if trash.exists() {
            home.safe_remove(&trash)?; // a leftover with the same name from an earlier run
        }
        fs::rename(&target.dir, &trash).map_err(|err| rename_error(&err, &target.dir))?;

        // 4. Config, then the moved folder, then data if asked.
        let mut report = UninstallReport {
            removed_dir: target.dir.clone(),
            ..UninstallReport::default()
        };
        for dir in config_dirs {
            if dir.exists() {
                home.safe_remove(&dir)?;
                report.removed_config.push(dir);
            }
        }
        if home.safe_remove(&trash).is_err() {
            // Not a runtime any more, but a file in it would not go (a program still has it open).
            report.leftover = Some(trash);
        }
        for dir in data_plan.delete {
            home.safe_remove(&dir)?;
            report.removed_data.push(dir);
        }
        report.kept_data = data_plan.keep;
        Ok(report)
    }
}

/// Which data folders to delete and which to keep.
struct DataPlan {
    delete: Vec<PathBuf>,
    keep: Vec<PathBuf>,
}

/// A data folder is deleted only if the user asked **and** no other installed version of the runtime still
/// names it. Folders that do not exist are not mentioned.
fn plan_data(
    home: &LunaHome,
    adapter: Option<&'static dyn RuntimeAdapter>,
    target: &InstalledRuntime,
    installed: &[InstalledRuntime],
    delete_data: bool,
) -> DataPlan {
    let mut plan = DataPlan {
        delete: Vec::new(),
        keep: Vec::new(),
    };
    let Some(adapter) = adapter else {
        return plan;
    };
    for dir in adapter.data_dirs(home, &target.version) {
        if !dir.exists() {
            continue;
        }
        let still_used = installed.iter().any(|other| {
            other.runtime == target.runtime
                && other.version != target.version
                && adapter.data_dirs(home, &other.version).contains(&dir)
        });
        if delete_data && !still_used {
            plan.delete.push(dir);
        } else {
            plan.keep.push(dir);
        }
    }
    plan
}

/// A failed rename of an install folder. On Windows a sharing violation (32) or an access denial (5) means a
/// program still has something open in the folder: the runtime is in use. Anything else is an ordinary error.
fn rename_error(err: &std::io::Error, dir: &std::path::Path) -> LunaError {
    let in_use = cfg!(windows) && matches!(err.raw_os_error(), Some(32 | 5));
    if in_use {
        LunaError::new(
            ErrorCode::RuntimeInUse,
            format!(
                "{} cannot be removed because a program is still using it ({err})",
                dir.display()
            ),
        )
        .with_details(json!({ "path": dir.display().to_string() }))
    } else {
        let shown = dir.display().to_string();
        LunaError::from(std::io::Error::new(err.kind(), format!("{shown}: {err}")))
            .with_details(json!({ "path": shown }))
    }
}
