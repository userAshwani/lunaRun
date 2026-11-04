//! PID files: `LUNA_HOME/run/<service-id>.pid`, one per running service (docs/SERVICES.md "No orphans").
//!
//! The file is written when a service starts and removed when it ends. If lunaRun dies without stopping its
//! services, the files are what the next launch finds: for each one it asks the OS whether that process is still
//! alive **and still the program that was started** (a number is reused), then offers to adopt or stop it, or
//! deletes the file when it is stale.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, Result};
use crate::paths::LunaHome;
use crate::state::{self, LoadOutcome, Persisted};

/// What is recorded about a started service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PidRecord {
    pub pid: u32,
    /// The program the service was started from, as it was started.
    pub exe: String,
    /// Seconds since 1970-01-01 UTC.
    pub started_at: u64,
    pub port: Option<u16>,
}

impl Persisted for PidRecord {
    const SCHEMA_VERSION: u32 = 1;
}

const EXTENSION: &str = "pid";

pub(super) fn path(home: &LunaHome, id: &str) -> PathBuf {
    home.run_dir().join(format!("{id}.{EXTENSION}"))
}

/// Record a started service. Atomic (a crash never leaves half a file).
pub(super) fn write(home: &LunaHome, id: &str, record: &PidRecord) -> Result<()> {
    fs::create_dir_all(home.run_dir()).map_err(|err| {
        crate::error::LunaError::new(
            ErrorCode::Io,
            format!("cannot create the run folder: {err}"),
        )
    })?;
    state::save(&path(home, id), record)
}

/// Delete the file. A missing file is fine; so is a failure to delete (the next launch checks it anyway).
pub(super) fn remove(home: &LunaHome, id: &str) {
    let _ = home.safe_remove(&path(home, id));
}

/// What the file says: `Ok(Some(record))` for a usable one, `Ok(None)` when there is nothing usable (no file, or a
/// damaged one, which is deleted together with the copy the state module sets aside, because a PID file says
/// nothing worth keeping once it cannot be read). `Err` for a file written by a newer lunaRun and for I/O errors:
/// those are left alone.
pub(super) fn read(home: &LunaHome, id: &str) -> Result<Option<PidRecord>> {
    let loaded = state::load::<PidRecord>(&path(home, id))?;
    Ok(match loaded.outcome {
        LoadOutcome::Fresh => None,
        LoadOutcome::Recovered { backup, .. } => {
            let _ = home.safe_remove(&backup);
            None
        }
        LoadOutcome::Loaded | LoadOutcome::Migrated { .. } => {
            (loaded.value.pid != 0).then_some(loaded.value)
        }
    })
}

/// The service ids that have a PID file.
pub(super) fn ids(home: &LunaHome) -> Vec<String> {
    let Ok(entries) = fs::read_dir(home.run_dir()) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().is_some_and(|ext| ext == EXTENSION))
                .then(|| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .flatten()
        })
        .collect();
    ids.sort();
    ids
}
