//! Starting and stopping a project: making its site answer, and taking it away again (FR-SV-01, FR-ST-03; D-039).
//!
//! A project is **started** when the user asks for it. That does three things, in this order, each only if it is not
//! already so: the database the project's `.env` expects (when that runtime is installed), the PHP pool of the PHP it
//! resolves to, and Caddy with the project in its Caddyfile (a running Caddy is given the change by a reload, with no
//! restart, so the other sites stay up). It is **all or none**: when a step fails or the user cancels, what this start
//! began is stopped again and the project is not served.
//!
//! **Stop** takes the project out of Caddy and stops only the services **this launcher started** and **no other started
//! project still needs**: a pool or Caddy the user started by hand, or one another project uses, is left running. When no
//! started project is left, Caddy stops too if the launcher started it.
//!
//! "Running" is never stored: it is read from the services ([`SiteEntry::running`]).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::dbadmin::{self, DatabaseCreated, DatabaseStatus};
use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};
use crate::jobs::{JobContext, JobManager, StartedJob};
use crate::projects::ProjectStore;
use crate::projects::envfix::{self, EnvFixApplied, PortChange};
use crate::services::{PortAllocator, PortPurpose, ServiceController, job_phase, pool_id};
use crate::sites::{SiteEntry, SiteManager, SiteState};

/// The key of a project's start or stop job: `project:<id>`.
pub fn job_key(project_id: &str) -> String {
    format!("project:{project_id}")
}

/// Start or stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectAction {
    Start,
    Stop,
}

impl ProjectAction {
    fn label_prefix(self) -> &'static str {
        match self {
            Self::Start => "Start project ",
            Self::Stop => "Stop project ",
        }
    }
}

/// How to connect to the database lunaRun runs for a project. The user is `root`; **there is no password** (the
/// databases of lunaRun accept connections from this computer only), and the `.env` password is never read (golden
/// rule 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseSettings {
    /// The service that runs it, `mysql@8.4`.
    pub service: String,
    /// `mysql` or `mariadb`: the value of `DB_CONNECTION`.
    pub runtime: String,
    pub host: String,
    /// The port lunaRun gave that service (the `.env` file may say another one).
    pub port: u16,
    /// `DB_DATABASE` of the `.env` file, when it names one.
    pub database: Option<String>,
    pub username: String,
}

/// Starts and stops projects. Cheap to clone; clones share what they remember.
#[derive(Clone)]
pub struct ProjectLauncher {
    store: ProjectStore,
    sites: SiteManager,
    controller: ServiceController,
    ports: PortAllocator,
    os: Os,
    /// The service rows **this launcher started**: only those are ever stopped by it.
    owned: Arc<Mutex<HashSet<String>>>,
}

fn no_site(entry: &SiteEntry) -> LunaError {
    let (reason, text) = match entry.state {
        SiteState::NoPhp => (
            "php",
            "its PHP is not installed (see the checklist of the project)",
        ),
        SiteState::FolderMissing => ("folder", "its folder is not there"),
        SiteState::Ready => ("none", "it has a site"),
    };
    LunaError::new(
        ErrorCode::NoSite,
        format!("the project cannot be started: {text}"),
    )
    .with_details(json!({ "projectId": entry.project_id, "reason": reason }))
}

fn cancelled() -> LunaError {
    LunaError::new(ErrorCode::Cancelled, "starting the project was cancelled")
}

fn unknown(id: &str) -> LunaError {
    LunaError::new(ErrorCode::NotFound, format!("no project with the id {id}"))
        .with_details(json!({ "id": id }))
}

impl ProjectLauncher {
    pub fn new(
        store: ProjectStore,
        sites: SiteManager,
        controller: ServiceController,
        ports: PortAllocator,
        os: Os,
    ) -> Self {
        Self {
            store,
            sites,
            controller,
            ports,
            os,
            owned: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn owns(&self, row: &str) -> bool {
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(row)
    }

    fn own(&self, row: &str) {
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(row.to_owned());
    }

    fn disown(&self, row: &str) {
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(row);
    }

    fn entry(&self, id: &str) -> Result<SiteEntry> {
        self.store.get(id)?;
        self.sites
            .plan()?
            .entries
            .into_iter()
            .find(|entry| entry.project_id == id)
            .ok_or_else(|| unknown(id))
    }

    /// Start `row` unless it already runs; remember that this launcher started it.
    async fn ensure(
        &self,
        row: &str,
        cancel: &CancellationToken,
        on_phase: &(dyn Fn(&str) + Sync),
        began: &mut Vec<String>,
    ) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if self.controller.is_running(row)? {
            return Ok(());
        }
        self.controller.start(row, cancel, on_phase).await?;
        self.own(row);
        began.push(row.to_owned());
        Ok(())
    }

    async fn start_stack(
        &self,
        entry: &SiteEntry,
        pool: &str,
        cancel: &CancellationToken,
        on_phase: &(dyn Fn(&str) + Sync),
        began: &mut Vec<String>,
    ) -> Result<()> {
        if let Some(database) = &entry.database_service {
            self.ensure(database, cancel, on_phase, began).await?;
        }
        self.ensure(pool, cancel, on_phase, began).await?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if self.controller.is_running("caddy")? {
            // Already up (started by hand or for another project): give it this project, without a restart.
            on_phase(job_phase::STARTING);
            self.sites.sync().await?;
        } else {
            self.ensure("caddy", cancel, on_phase, began).await?;
        }
        Ok(())
    }

    /// Start the project: its database, its PHP pool and Caddy, each only when it is not running, and Caddy serves it.
    /// `on_phase` hears `setup` (a database's first start) and `starting`.
    ///
    /// # Errors
    /// `NOT_FOUND` (no such project, or a service of it whose runtime is not installed: see Runtimes), `NO_SITE` (its
    /// PHP is not installed or its folder is gone), `BUSY` while the app quits, `PORT_IN_USE`, `CONFIG_INVALID`,
    /// `RELOAD_FAILED`, `DATABASE_INIT_FAILED`, `SERVICE_START_FAILED`, `CANCELLED`. A failed start leaves nothing it
    /// began running and the project not served.
    pub async fn start(
        &self,
        id: &str,
        cancel: &CancellationToken,
        on_phase: &(dyn Fn(&str) + Sync),
    ) -> Result<()> {
        let entry = self.entry(id)?;
        if entry.state != SiteState::Ready {
            return Err(no_site(&entry));
        }
        let version = entry
            .php_version
            .clone()
            .ok_or_else(|| LunaError::internal("a site that is ready has no PHP"))?;
        let pool = pool_id(self.os, &version);
        self.sites.serve(id);
        let mut began = Vec::new();
        match self
            .start_stack(&entry, &pool, cancel, on_phase, &mut began)
            .await
        {
            Ok(()) => Ok(()),
            Err(err) => {
                // All or none: what this start began is stopped, and the project is not served.
                self.sites.unserve(id);
                for row in began.iter().rev() {
                    let _ = self.controller.stop(row).await;
                    self.disown(row);
                }
                let _ = self.sites.sync().await;
                Err(err)
            }
        }
    }

    /// Stop the project: Caddy stops serving it, and the services this launcher started for it are stopped unless
    /// another started project needs them (the pool of the same PHP, the same database, Caddy itself).
    ///
    /// # Errors
    /// `NOT_FOUND` for an unknown project, `CONFIG_INVALID` or `RELOAD_FAILED` when Caddy cannot take the change, and
    /// the first error of a stop. Stopping what is not running is a success.
    pub async fn stop(&self, id: &str) -> Result<()> {
        let entry = self.entry(id)?;
        self.sites.unserve(id);
        let others: Vec<SiteEntry> = self
            .sites
            .plan()?
            .entries
            .into_iter()
            .filter(|other| other.served && other.state == SiteState::Ready)
            .collect();
        let mut first_error: Option<LunaError> = None;
        let mut note = |result: Result<()>| {
            if let Err(err) = result {
                first_error.get_or_insert(err);
            }
        };

        // Caddy first: this project is no longer served, whatever else happens.
        if others.is_empty() && self.owns("caddy") {
            note(self.controller.stop("caddy").await);
            self.disown("caddy");
        } else {
            note(self.sites.sync().await.map(|_| ()));
        }
        if let Some(version) = &entry.php_version {
            let pool = pool_id(self.os, version);
            let needed = others
                .iter()
                .any(|other| other.php_version.as_ref() == Some(version));
            if self.owns(&pool) && !needed {
                note(self.controller.stop(&pool).await);
                self.disown(&pool);
            }
        }
        if let Some(database) = &entry.database_service {
            let needed = others
                .iter()
                .any(|other| other.database_service.as_ref() == Some(database));
            if self.owns(database) && !needed {
                note(self.controller.stop(database).await);
                self.disown(database);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Start or stop as a background job `project:<id>` (progress, Cancel). Asking again for the action that is
    /// already running returns that job; the other action while one runs is `BUSY`.
    ///
    /// # Errors
    /// `NOT_FOUND` for an unknown project, `BUSY` when the other action's job is running.
    pub fn run(&self, jobs: &JobManager, id: &str, action: ProjectAction) -> Result<StartedJob> {
        let project = self.store.get(id)?;
        let label = format!("{}{}", action.label_prefix(), project.name);
        let launcher = self.clone();
        let project_id = id.to_owned();
        let started = jobs.start(&job_key(id), &label, move |ctx: JobContext| async move {
            let cancel = ctx.cancel_token().clone();
            let on_phase = |phase: &str| ctx.phase(phase, None);
            match action {
                ProjectAction::Start => launcher.start(&project_id, &cancel, &on_phase).await,
                ProjectAction::Stop => {
                    on_phase(job_phase::STOPPING);
                    launcher.stop(&project_id).await
                }
            }
        });
        if started.already_running && jobs.get(&started.id).is_some_and(|job| job.label != label) {
            return Err(LunaError::new(
                ErrorCode::Busy,
                format!(
                    "{} is busy with another action: wait for it to finish",
                    project.name
                ),
            )
            .with_details(json!({ "projectId": id })));
        }
        Ok(started)
    }

    /// The settings to connect to the database lunaRun runs for this project, or `None` when the project expects no
    /// MySQL or MariaDB or that runtime is not installed. Asking assigns the database's port (the one its start will
    /// use), which is stable from then on.
    ///
    /// # Errors
    /// `NOT_FOUND` for an unknown project, `NO_FREE_PORT` and state-file errors.
    pub fn database_settings(&self, id: &str) -> Result<Option<DatabaseSettings>> {
        let project = self.store.get(id)?;
        if !Path::new(&project.path).is_dir() {
            return Ok(None);
        }
        let (report, resolution) = self.sites.evaluate(&project)?;
        let Some(suggestion) = resolution.database else {
            return Ok(None);
        };
        let (Some(runtime), Some(line), Some(_installed)) = (
            suggestion.runtime,
            suggestion.line,
            suggestion.installed_version,
        ) else {
            return Ok(None);
        };
        let service = format!("{runtime}@{line}");
        let port = self.ports.assign(&service, PortPurpose::Database)?.port;
        Ok(Some(DatabaseSettings {
            service,
            runtime,
            host: "127.0.0.1".to_owned(),
            port,
            database: report.database.and_then(|hint| hint.database),
            username: "root".to_owned(),
        }))
    }

    // -----------------------------------------------------------------------------------------
    // The fixes of the database settings (FR-PJ-05, D-041)
    // -----------------------------------------------------------------------------------------

    /// The change of `DB_PORT` in the project's `.env` that would make it connect to the database lunaRun runs, as a
    /// preview: the one line that changes and the backup that would be made. `None` when there is nothing to fix (the
    /// project expects no database lunaRun runs, has no `.env` file, or its `DB_PORT` is already right). Writes nothing.
    ///
    /// # Errors
    /// `NOT_FOUND` for an unknown project, `ENV_CHANGED` for a `.env` that cannot be edited safely, and what
    /// [`ProjectLauncher::database_settings`] answers with.
    pub fn env_port_preview(&self, id: &str) -> Result<Option<PortChange>> {
        let project = self.store.get(id)?;
        let Some(settings) = self.database_settings(id)? else {
            return Ok(None);
        };
        envfix::plan_port(Path::new(&project.path), settings.port)
    }

    /// Make the change the user confirmed: a backup of the `.env` file, then the one line. `expected` is the preview the
    /// page showed; when the port or the file is not what it showed, nothing is written (`ENV_CHANGED`).
    ///
    /// # Errors
    /// As [`ProjectLauncher::env_port_preview`], and `ENV_CHANGED` for anything that is not the preview.
    pub fn env_port_apply(&self, id: &str, expected: &PortChange) -> Result<EnvFixApplied> {
        let project = self.store.get(id)?;
        // The port is lunaRun's own, not the page's: a preview for another port is not applied.
        let settings = self
            .database_settings(id)?
            .filter(|s| s.port == expected.port);
        if settings.is_none() {
            return Err(LunaError::new(
                ErrorCode::EnvChanged,
                "the database lunaRun runs for this project is not the one in the preview",
            )
            .with_details(json!({ "reason": "changed", "projectId": id })));
        }
        envfix::apply_port(Path::new(&project.path), expected)
    }

    /// The database client of the installed runtime of `settings`.
    fn database_client(&self, settings: &DatabaseSettings) -> Result<std::path::PathBuf> {
        let runtime = self.controller.runtime_of(&settings.service)?;
        crate::runtimes::adapter_for(&runtime.runtime)
            .and_then(|adapter| {
                adapter
                    .binaries(&runtime.dir, self.os)
                    .into_iter()
                    .find(|binary| binary.name == settings.runtime)
                    .map(|binary| binary.path)
            })
            .ok_or_else(|| {
                LunaError::internal(format!("{} has no database client", settings.runtime))
            })
    }

    /// Where the database the project's `.env` names stands. Asks the server only when it runs.
    ///
    /// # Errors
    /// `NOT_FOUND` for an unknown project, and what the client answers with (`DATABASE_COMMAND_FAILED`).
    pub fn database_status(&self, id: &str) -> Result<DatabaseStatus> {
        let Some(settings) = self.database_settings(id)? else {
            return Ok(DatabaseStatus::NotApplicable);
        };
        let Some(name) = settings.database.clone() else {
            return Ok(DatabaseStatus::NoName);
        };
        if !dbadmin::is_valid_name(&name) {
            return Ok(DatabaseStatus::InvalidName);
        }
        if !self.controller.is_running(&settings.service)? {
            return Ok(DatabaseStatus::NotRunning);
        }
        let client = self.database_client(&settings)?;
        let present = dbadmin::exists(&client, &settings.runtime, settings.port, &name)?;
        Ok(if present {
            DatabaseStatus::Exists
        } else {
            DatabaseStatus::Missing
        })
    }

    /// Create the database the project's `.env` names, on the server lunaRun runs for it. Nothing is changed when it exists.
    ///
    /// # Errors
    /// `NO_SITE` is not used here: a project with nothing to create answers `INVALID_INPUT` (`details.field` `database`),
    /// `DATABASE_NOT_RUNNING` when the server is not running, and `DATABASE_COMMAND_FAILED`.
    pub fn database_create(&self, id: &str) -> Result<DatabaseCreated> {
        let settings = self.database_settings(id)?.ok_or_else(|| {
            LunaError::new(
                ErrorCode::InvalidInput,
                "this project expects no database that lunaRun runs",
            )
            .with_details(json!({ "field": "database" }))
        })?;
        let name = settings.database.clone().ok_or_else(|| {
            LunaError::new(
                ErrorCode::InvalidInput,
                "the .env file of this project names no DB_DATABASE",
            )
            .with_details(json!({ "field": "database" }))
        })?;
        if !self.controller.is_running(&settings.service)? {
            return Err(LunaError::new(
                ErrorCode::DatabaseNotRunning,
                format!(
                    "{} is not running: start the project first",
                    settings.service
                ),
            )
            .with_details(json!({ "service": settings.service })));
        }
        let client = self.database_client(&settings)?;
        let created = dbadmin::create(&client, &settings.runtime, settings.port, &name)?;
        Ok(DatabaseCreated { name, created })
    }
}
