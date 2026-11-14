//! The sites of the projects: which project is served on which port, and what Caddy is told (FR-ST-01 to FR-ST-03;
//! docs/SERVICES.md section 4; D-038).
//!
//! A project **has a site** when it resolves to a PHP that is **installed** and its folder exists. The site is
//! `http://127.0.0.1:<port>`: a port the allocator keeps for the project (`site@<project id>`, the same one every time,
//! FR-ST-01), the project's document root, and the FastCGI ports of the pool of the PHP it resolves to (the ports that
//! pool gets when it is started: [`pool_ports`]).
//!
//! [`SiteManager::sync`] writes the Caddyfile for all of them and, when Caddy is running, validates it and reloads it
//! **without a restart** (FR-ST-03); a Caddyfile that fails validation never reaches the running Caddy. It is called
//! after every change that can alter a site: a project added, removed or changed. [`SiteManager::view`] is what the
//! page shows, honestly: a site that is served, a project with no PHP installed (no site yet), a folder that has gone.
//!
//! Nothing is written into a project folder: its path is only text in the Caddyfile (golden rule 3).

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::diagnostics::{Arch, Os};
use crate::error::{LunaError, Result};
use crate::paths::LunaHome;
use crate::projects::{DetectionData, Project, ProjectStore, Surroundings, resolve_project};
use crate::registry::{CalDate, RegistryHolder};
use crate::runtimes::list_installed;
use crate::services::{
    CaddyDefinition, PortAllocator, PortPurpose, SITE_PREFIX, Site, Supervisor, caddy_definition,
    pool_ports, web_server_runtime,
};

/// `caddy validate` and `caddy reload` read one small file and talk to a local port.
const CADDY_LIMIT: Duration = Duration::from_secs(30);
/// A site's name is a slug of the project's name (letters and digits) and the start of its id.
const SLUG_CHARS: usize = 20;

/// Why a project is, or is not, served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SiteState {
    /// It has a site: a port, a document root and a PHP pool.
    Ready,
    /// The PHP it resolves to is not installed (or nothing fits): no site until there is one.
    NoPhp,
    /// The project's folder is not there.
    FolderMissing,
}

/// One project and its site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SiteEntry {
    pub project_id: String,
    pub state: SiteState,
    /// The name of the site: its access log's name and its tab on the Services page.
    pub name: String,
    pub port: Option<u16>,
    /// `http://127.0.0.1:8101`, when the project has a site.
    pub address: Option<String>,
    /// The PHP the site runs on, `8.3.14`, when it has one.
    pub php_version: Option<String>,
}

/// Is there a web server to serve the sites?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WebServer {
    /// Caddy is not installed: the sites are planned, and nothing serves them until it is.
    NotInstalled,
    /// Caddy is installed and not running: the sites answer once it is started (Services page).
    Stopped,
    Running,
}

/// What the page shows about the sites.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SitesView {
    /// One per project, in the order the projects were added.
    pub sites: Vec<SiteEntry>,
    pub web_server: WebServer,
    /// Why the last change could not be given to Caddy (it rejected the configuration, or could not reload it); the
    /// running Caddy keeps what it had. `None` when the last change went through.
    pub problem: Option<LunaError>,
}

/// The sites of every project, and the entries to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitePlan {
    pub entries: Vec<SiteEntry>,
    /// The sites Caddy serves: the `Ready` entries, in the same order.
    pub sites: Vec<Site>,
}

/// What a [`sync`](SiteManager::sync) did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Caddy is not installed: nothing was written.
    NoWebServer,
    /// The Caddyfile is written; Caddy is not running and reads it when it starts.
    Written,
    /// Caddy is running and already has this Caddyfile.
    Unchanged,
    /// Caddy is running: it validated the new Caddyfile and loaded it, without a restart.
    Reloaded,
}

type DefinitionHook = Arc<dyn Fn(&mut CaddyDefinition) + Send + Sync>;

/// Plans and applies the sites. Cheap to clone; clones share what they remember.
#[derive(Clone)]
pub struct SiteManager {
    home: LunaHome,
    store: ProjectStore,
    data: Arc<DetectionData>,
    registry: RegistryHolder,
    ports: PortAllocator,
    supervisor: Supervisor,
    os: Os,
    hook: Option<DefinitionHook>,
    /// The Caddyfile text the running Caddy last accepted (this session), to skip a reload that changes nothing.
    applied: Arc<Mutex<Option<String>>>,
    problem: Arc<Mutex<Option<LunaError>>>,
    /// One sync at a time: two would race on the Caddyfile and on the reload.
    gate: Arc<tokio::sync::Mutex<()>>,
}

/// `Shop (café)` and id `4b1e0c…` become `shop-caf-4b1e0c`: ASCII letters, digits and `-`, unique by the id.
fn site_name(project: &Project) -> String {
    let mut slug = String::new();
    for c in project.name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(SLUG_CHARS).collect();
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "site" } else { slug };
    let short: String = project.id.chars().take(6).collect();
    format!("{slug}-{short}")
}

fn site_key(project: &Project) -> String {
    format!("{SITE_PREFIX}{}", project.id)
}

impl SiteManager {
    /// `os` is a parameter so both variants are tested on any machine. Pass `Os::current()`.
    pub fn new(
        home: LunaHome,
        store: ProjectStore,
        data: Arc<DetectionData>,
        registry: RegistryHolder,
        ports: PortAllocator,
        supervisor: Supervisor,
        os: Os,
    ) -> Self {
        Self {
            home,
            store,
            data,
            registry,
            ports,
            supervisor,
            os,
            hook: None,
            applied: Arc::new(Mutex::new(None)),
            problem: Arc::new(Mutex::new(None)),
            gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Change the Caddy definition before it is validated and reloaded. For tests: the real Caddy is not there.
    #[must_use]
    pub fn with_definition_hook(
        mut self,
        hook: impl Fn(&mut CaddyDefinition) + Send + Sync + 'static,
    ) -> Self {
        self.hook = Some(Arc::new(hook));
        self
    }

    /// The site of every project. Reads each project folder (read-only) and asks the allocator for ports: the first
    /// time a project gets a site its port is chosen and saved, which is the one write.
    ///
    /// # Errors
    /// `NO_FREE_PORT` and I/O and state-file errors; a project whose files cannot be read is not an error (its scan
    /// carries warnings and it simply has no PHP requirement).
    pub fn plan(&self) -> Result<SitePlan> {
        let installed = list_installed(&self.home)?;
        let today = CalDate::today()
            .ok_or_else(|| LunaError::internal("the system clock does not give a usable date"))?;
        let available = self
            .registry
            .available(self.os, Arch::current(), today)
            .runtimes;
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: self.os,
        };

        let mut entries = Vec::new();
        let mut sites = Vec::new();
        for project in self.store.list() {
            let name = site_name(&project);
            let folder = Path::new(&project.path);
            let mut entry = SiteEntry {
                project_id: project.id.clone(),
                state: SiteState::FolderMissing,
                name: name.clone(),
                port: None,
                address: None,
                php_version: None,
            };
            if !folder.is_dir() {
                entries.push(entry);
                continue;
            }
            let report = crate::projects::detect(folder, &self.data);
            let resolution = resolve_project(&project, &report, &self.data, &around);
            let php = resolution
                .php
                .recommended
                .filter(|chosen| chosen.installed && !chosen.needs_install);
            let Some(php) = php else {
                entry.state = SiteState::NoPhp;
                entries.push(entry);
                continue;
            };
            // What the page and a rescan say, not a stale copy: a document root the user did not choose follows the folder.
            let doc_root = if project.doc_root_custom {
                project.doc_root.clone()
            } else {
                report.doc_root.path
            };
            let root = if doc_root == "." {
                folder.to_path_buf()
            } else {
                folder.join(&doc_root)
            };
            let port = self
                .ports
                .assign(&site_key(&project), PortPurpose::Site)?
                .port;
            sites.push(Site {
                name,
                port,
                root,
                php_ports: pool_ports(&self.ports, self.os, &php.version)?,
            });
            entry.state = SiteState::Ready;
            entry.port = Some(port);
            entry.address = Some(format!("http://127.0.0.1:{port}"));
            entry.php_version = Some(php.version);
            entries.push(entry);
        }
        Ok(SitePlan { entries, sites })
    }

    /// The sites Caddy serves, for [`ServiceController::with_sites`](crate::services::ServiceController::with_sites).
    ///
    /// # Errors
    /// What [`plan`](Self::plan) refuses with.
    pub fn sites(&self) -> Result<Vec<Site>> {
        Ok(self.plan()?.sites)
    }

    /// What the page shows: each project's site, whether there is a web server to serve it, and the last problem.
    ///
    /// # Errors
    /// What [`plan`](Self::plan) refuses with.
    pub fn view(&self) -> Result<SitesView> {
        let entries = self.plan()?.entries;
        let installed = list_installed(&self.home)?;
        let web_server = if web_server_runtime(&installed).is_none() {
            WebServer::NotInstalled
        } else if self
            .supervisor
            .status("caddy")
            .is_some_and(|status| status.state.has_process())
        {
            WebServer::Running
        } else {
            WebServer::Stopped
        };
        Ok(SitesView {
            sites: entries,
            web_server,
            problem: self
                .problem
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        })
    }

    /// Write the Caddyfile for the sites as they are now and, if Caddy is running, give it the change: validate, then
    /// reload, with no restart, so the other sites stay up (FR-ST-03). The ports of projects that no longer exist are
    /// released. The outcome (a problem, or none) is remembered for [`view`](Self::view).
    ///
    /// # Errors
    /// `CONFIG_INVALID` (Caddy rejected the file: the running Caddy is untouched), `RELOAD_FAILED`, and what
    /// [`plan`](Self::plan) refuses with.
    pub async fn sync(&self) -> Result<SyncOutcome> {
        let _one_at_a_time = self.gate.lock().await;
        let outcome = self.apply().await;
        *self.problem.lock().unwrap_or_else(PoisonError::into_inner) =
            outcome.as_ref().err().cloned();
        outcome
    }

    async fn apply(&self) -> Result<SyncOutcome> {
        let this = self.clone();
        let (plan, caddy_file) = tokio::task::spawn_blocking(move || -> Result<_> {
            let plan = this.plan()?;
            this.release_gone_projects()?;
            let installed = list_installed(&this.home)?;
            let definition = match web_server_runtime(&installed) {
                Some(runtime) => {
                    let mut definition =
                        caddy_definition(&this.home, &this.ports, runtime, &plan.sites, this.os)?;
                    if let Some(hook) = &this.hook {
                        hook(&mut definition);
                    }
                    Some(definition)
                }
                None => None,
            };
            let text = definition
                .as_ref()
                .and_then(|d| fs::read_to_string(&d.config_file).ok());
            Ok((definition, text))
        })
        .await
        .map_err(|err| LunaError::internal(format!("planning the sites failed: {err}")))??;

        let Some(definition) = plan else {
            return Ok(SyncOutcome::NoWebServer);
        };
        let running = self
            .supervisor
            .status("caddy")
            .is_some_and(|status| status.state.has_process());
        if !running {
            return Ok(SyncOutcome::Written);
        }
        let text = caddy_file.unwrap_or_default();
        if self
            .applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_deref()
            == Some(text.as_str())
        {
            return Ok(SyncOutcome::Unchanged);
        }
        let cancel = CancellationToken::new();
        definition.validate(CADDY_LIMIT, &cancel).await?;
        definition.reload(CADDY_LIMIT, &cancel).await?;
        *self.applied.lock().unwrap_or_else(PoisonError::into_inner) = Some(text);
        Ok(SyncOutcome::Reloaded)
    }

    /// The port of a project that was removed is free for another.
    fn release_gone_projects(&self) -> Result<()> {
        let alive: Vec<String> = self.store.list().iter().map(site_key).collect();
        for (key, _) in self.ports.all()? {
            if key.starts_with(SITE_PREFIX) && !alive.contains(&key) {
                self.ports.release(&key)?;
            }
        }
        Ok(())
    }
}
