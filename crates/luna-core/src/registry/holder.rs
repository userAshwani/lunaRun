//! The registry the running app uses, and how it got there.
//!
//! The app starts with the copy built into it, so the Runtimes page has something to show at once, then loads
//! the fresh list in the background ([`RegistryHolder::refresh`]: network, then the saved copy, then the built-in
//! one, see `load.rs`) and swaps it in. [`RegistryStatus`] tells the page where the list in use came from and
//! why it is not the freshest, so it can say "you appear to be offline" or "update lunaRun".

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::load::{RegistrySource, bundled_registry, load_registry};
use super::{CalDate, Registry, RuntimeCatalog};
use crate::diagnostics::{Arch, Os};
use crate::download::Downloader;
use crate::error::{LunaError, Result};
use crate::paths::LunaHome;

/// Where the list in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RegistrySourceKind {
    /// Just downloaded.
    Fresh,
    /// The copy saved by an earlier run.
    Cache,
    /// The list built into this version of lunaRun.
    Bundled,
}

/// What the Runtimes page needs to explain its list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RegistryStatus {
    pub source: RegistrySourceKind,
    /// How old the saved copy is, for `cache` when the file system says.
    #[ts(type = "number | null")]
    pub age_seconds: Option<u64>,
    /// Why a fresher list was not used: offline, an HTTP error, or `REGISTRY_UNSUPPORTED_SCHEMA` (update lunaRun).
    pub problem: Option<LunaError>,
    /// A refresh is running right now.
    pub refreshing: bool,
}

/// The answer to "what can I install here": every runtime with its versions, and the status of the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimesAvailable {
    pub runtimes: Vec<RuntimeCatalog>,
    pub registry: RegistryStatus,
}

#[derive(Clone)]
struct Snapshot {
    registry: Arc<Registry>,
    source: RegistrySource,
    problem: Option<LunaError>,
}

struct Inner {
    state: RwLock<Snapshot>,
    refreshing: AtomicBool,
}

/// Holds the registry in use. Cheap to clone, safe to share between threads.
#[derive(Clone)]
pub struct RegistryHolder(Arc<Inner>);

/// A lock that survives another thread having panicked while holding it: the snapshot is replaced as a
/// whole, so it is never half written.
fn read(lock: &RwLock<Snapshot>) -> RwLockReadGuard<'_, Snapshot> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write(lock: &RwLock<Snapshot>) -> RwLockWriteGuard<'_, Snapshot> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Clears the "refreshing" flag however the refresh ends, including by panic.
struct RefreshGuard<'a>(&'a AtomicBool);

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl RegistryHolder {
    /// Start with the list built into this version, until [`refresh`](Self::refresh) has something better.
    pub fn with_bundled() -> Result<Self> {
        Ok(Self(Arc::new(Inner {
            state: RwLock::new(Snapshot {
                registry: Arc::new(bundled_registry()?),
                source: RegistrySource::Bundled,
                problem: None,
            }),
            refreshing: AtomicBool::new(false),
        })))
    }

    /// The registry in use. Holding it does not block a refresh.
    pub fn registry(&self) -> Arc<Registry> {
        Arc::clone(&read(&self.0.state).registry)
    }

    pub fn status(&self) -> RegistryStatus {
        let snapshot = read(&self.0.state);
        let (source, age_seconds) = match &snapshot.source {
            RegistrySource::Fresh => (RegistrySourceKind::Fresh, None),
            RegistrySource::Cache { age } => (RegistrySourceKind::Cache, age.map(|a| a.as_secs())),
            RegistrySource::Bundled => (RegistrySourceKind::Bundled, None),
        };
        RegistryStatus {
            source,
            age_seconds,
            problem: snapshot.problem.clone(),
            refreshing: self.0.refreshing.load(Ordering::Acquire),
        }
    }

    /// The Runtimes page's data for this computer as of `today`.
    pub fn available(&self, os: Os, arch: Arch, today: CalDate) -> RuntimesAvailable {
        RuntimesAvailable {
            runtimes: self.registry().catalog(os, arch, today),
            registry: self.status(),
        }
    }

    /// Load the best registry available (network, saved copy, built-in copy) and use it from now on. A refresh
    /// that is already running is not started twice: the caller gets the current status.
    pub async fn refresh(
        &self,
        home: &LunaHome,
        downloader: &Downloader,
        location: &str,
    ) -> Result<RegistryStatus> {
        if self.0.refreshing.swap(true, Ordering::AcqRel) {
            return Ok(self.status());
        }
        let _guard = RefreshGuard(&self.0.refreshing);

        let loaded = load_registry(home, downloader, location).await?;
        *write(&self.0.state) = Snapshot {
            registry: Arc::new(loaded.registry),
            source: loaded.source,
            problem: loaded.problem,
        };
        // The flag is still set here: report the state the page will see after the guard clears it.
        let mut status = self.status();
        status.refreshing = false;
        Ok(status)
    }
}
