//! What is installed: `LUNA_HOME/runtimes/<runtime>/<version>/` plus its `install.json` receipt
//! (FR-RT-13, docs/RUNTIMES.md section 4).
//!
//! The receipt is written last, after the files are in place, so **a folder without a valid receipt is not
//! an installed runtime**: it is a half-finished or foreign folder and is listed as nothing. Listing is
//! read-only, it never moves, repairs or deletes anything (cleaning up incomplete installs is the
//! installer's job at startup).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::diagnostics::{Arch, Os};
use crate::error::{IoResultExt, Result};
use crate::paths::LunaHome;
use crate::registry::{Artifact, RuntimeVersion};
use crate::state::{self, Persisted};

/// File name of the receipt inside a runtime's folder.
pub const RECEIPT_FILE: &str = "install.json";

/// One downloaded part of an install, as it was when installed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReceiptArtifact {
    /// Build flavour such as `nts-vs16` or `static-fpm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub variant: Option<String>,
    pub url: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub size: u64,
}

impl From<&Artifact> for ReceiptArtifact {
    fn from(artifact: &Artifact) -> Self {
        Self {
            variant: artifact.variant.clone(),
            url: artifact.url.clone(),
            sha256: artifact.sha256.clone(),
            size: artifact.size,
        }
    }
}

/// The `install.json` receipt. Facts about the install, nothing that changes while it is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallReceipt {
    pub runtime: String,
    pub version: String,
    /// `windows`, `macos` or `linux`, as the registry spells them.
    pub os: String,
    /// `x86_64` or `aarch64`.
    pub arch: String,
    /// An Intel build installed on an Apple silicon Mac (it runs under Rosetta).
    #[serde(default)]
    pub rosetta: bool,
    /// Seconds since 1970-01-01 UTC.
    #[ts(type = "number")]
    pub installed_at: u64,
    pub artifacts: Vec<ReceiptArtifact>,
}

impl Persisted for InstallReceipt {
    const SCHEMA_VERSION: u32 = 1;
}

impl InstallReceipt {
    pub fn new(
        runtime: &str,
        version: &str,
        os: Os,
        arch: Arch,
        rosetta: bool,
        artifacts: &[&Artifact],
    ) -> Self {
        Self {
            runtime: runtime.to_owned(),
            version: version.to_owned(),
            os: os_name(os).to_owned(),
            arch: arch_name(arch).to_owned(),
            rosetta,
            installed_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            artifacts: artifacts
                .iter()
                .map(|a| ReceiptArtifact::from(*a))
                .collect(),
        }
    }

    /// Write the receipt into an install folder (atomically).
    pub fn write(&self, dir: &Path) -> Result<()> {
        state::save(&dir.join(RECEIPT_FILE), self)
    }
}

pub(crate) fn os_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "windows",
        Os::Macos => "macos",
        Os::Linux => "linux",
        // Nothing can be installed on a platform the registry has no builds for; the name only labels a receipt.
        Os::Unknown => "unknown",
    }
}

pub(crate) fn arch_name(arch: Arch) -> &'static str {
    match arch {
        Arch::X86_64 => "x86_64",
        Arch::Aarch64 => "aarch64",
        Arch::Unknown => "unknown",
    }
}

/// An installed runtime as the Runtimes page lists it: plain data, no paths as `PathBuf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledView {
    pub runtime: String,
    pub version: String,
    /// The install folder, for "show in folder" and for diagnostics.
    pub dir: String,
    /// Seconds since 1970-01-01 UTC.
    #[ts(type = "number")]
    pub installed_at: u64,
    /// An Intel build running under Rosetta on an Apple silicon Mac.
    pub rosetta: bool,
    /// How many downloads the install was made of (PHP on macOS and Linux: two).
    pub parts: usize,
}

impl From<&InstalledRuntime> for InstalledView {
    fn from(installed: &InstalledRuntime) -> Self {
        Self {
            runtime: installed.runtime.clone(),
            version: installed.version.clone(),
            dir: installed.dir.display().to_string(),
            installed_at: installed.receipt.installed_at,
            rosetta: installed.receipt.rosetta,
            parts: installed.receipt.artifacts.len(),
        }
    }
}

/// A runtime that is installed and has a valid receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRuntime {
    pub runtime: String,
    pub version: String,
    pub dir: PathBuf,
    pub receipt: InstallReceipt,
}

/// The receipt in `dir`, or `None` when there is none or it cannot be trusted: missing, not JSON, a
/// schema from a newer lunaRun, or the wrong shape. Read-only, unlike [`state::load`], which moves a
/// damaged file aside.
fn read_receipt(dir: &Path) -> Option<InstallReceipt> {
    let bytes = fs::read(dir.join(RECEIPT_FILE)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let schema = value.get("schema")?.as_u64()?;
    if schema != u64::from(InstallReceipt::SCHEMA_VERSION) {
        return None;
    }
    serde_json::from_value(value).ok()
}

fn directories(path: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None; // never follow a link planted where a folder should be
            }
            let name = entry.file_name().to_str()?.to_owned();
            // `.staging-*`, `.trash` and the like are the installer's own work folders.
            (!name.starts_with('.')).then(|| (name, entry.path()))
        })
        .collect()
}

/// Every installed runtime: runtimes in name order, newest version first.
///
/// A version folder counts only if it has a valid receipt whose runtime and version match the folder
/// names (a copied or renamed folder does not).
pub fn list_installed(home: &LunaHome) -> Result<Vec<InstalledRuntime>> {
    let root = home.runtimes_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }
    // A `LUNA_HOME` we cannot read at all is an error. Anything below it that looks wrong is just skipped.
    fs::read_dir(&root).at_path(&root)?;

    let mut found = Vec::new();
    let mut runtimes = directories(&root);
    runtimes.sort();
    for (runtime, runtime_dir) in runtimes {
        let mut versions: Vec<(RuntimeVersion, String, PathBuf)> = directories(&runtime_dir)
            .into_iter()
            .filter_map(|(name, dir)| Some((RuntimeVersion::parse(&name)?, name, dir)))
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, name, dir) in versions {
            let Some(receipt) = read_receipt(&dir) else {
                continue;
            };
            if receipt.runtime != runtime || receipt.version != name {
                continue;
            }
            found.push(InstalledRuntime {
                runtime: runtime.clone(),
                version: name,
                dir,
                receipt,
            });
        }
    }
    Ok(found)
}
