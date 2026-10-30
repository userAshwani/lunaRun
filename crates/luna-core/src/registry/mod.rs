//! The registry: which versions of PHP, Node.js, MySQL and so on exist, and where to download them
//! (docs/RUNTIMES.md section 3, FR-RT-01, FR-RT-07, NFR-05).
//!
//! This module only **parses and validates** a registry and answers questions about it. Fetching,
//! caching and the bundled fallback are separate (M1.2 task 2). The rules:
//!
//! - **Forward compatible.** Unknown *fields* are ignored. Artifacts or versions this build does not
//!   understand (a new OS, a new archive format, a new channel) are skipped, never an error, so an older
//!   lunaRun keeps working with a newer registry. A registry whose `schema` is newer than supported is
//!   refused with `REGISTRY_UNSUPPORTED_SCHEMA`, so the app can tell the user to update.
//! - **Strict about what it does use.** A usable artifact needs an `https://` URL, a 64-character
//!   SHA-256 and a non-zero size, and names that become folder names (runtime ids, versions) must be
//!   safe path components. Anything else is `REGISTRY_INVALID`, with a message naming the culprit.

mod version;

use std::collections::{BTreeMap, HashSet};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::diagnostics::{Arch, Os};
use crate::error::{ErrorCode, LunaError, Result};
use crate::extract::ArchiveFormat;
use crate::paths::component;

pub use version::{CalDate, RuntimeVersion};

/// The newest registry `schema` this build understands.
pub const SUPPORTED_SCHEMA: u32 = 1;

fn invalid(message: impl Into<String>) -> LunaError {
    LunaError::new(ErrorCode::RegistryInvalid, message)
}

// ---------------------------------------------------------------------------------------------
// The file as written (serde). Every field the registry may grow is optional or defaulted.
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRegistry {
    #[serde(default)]
    runtimes: BTreeMap<String, RawRuntime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRuntime {
    display_name: String,
    #[serde(default)]
    license: Option<License>,
    #[serde(default)]
    lines: BTreeMap<String, RawLine>,
    #[serde(default)]
    versions: Vec<RawVersion>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLine {
    #[serde(default)]
    eol: Option<String>,
    #[serde(default)]
    security_only_since: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawVersion {
    version: String,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    released_at: Option<String>,
    #[serde(default)]
    artifacts: Vec<RawArtifact>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawArtifact {
    os: String,
    arch: String,
    #[serde(default)]
    variant: Option<String>,
    format: String,
    url: String,
    sha256: String,
    size: u64,
    #[serde(default)]
    strip_components: usize,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// The validated registry
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct License {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Channel {
    Stable,
    Rc,
    Beta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactFormat {
    Zip,
    TarGz,
    TarXz,
    /// A single file to place as is, such as `composer.phar`.
    File,
}

impl ArtifactFormat {
    /// The extraction format, or `None` for a single file that is not an archive.
    pub fn archive_format(self) -> Option<ArchiveFormat> {
        match self {
            Self::Zip => Some(ArchiveFormat::Zip),
            Self::TarGz => Some(ArchiveFormat::TarGz),
            Self::TarXz => Some(ArchiveFormat::TarXz),
            Self::File => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub os: Os,
    pub arch: Arch,
    /// Build flavour such as `nts-vs16`, if the registry distinguishes any.
    pub variant: Option<String>,
    pub format: ArtifactFormat,
    pub url: String,
    /// Lowercase hexadecimal.
    pub sha256: String,
    pub size: u64,
    pub strip_components: usize,
    /// Prerequisites to check first, such as `vcredist-14-x64`.
    pub requires: Vec<String>,
    /// Extensions built into a static PHP, empty when not applicable.
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: RuntimeVersion,
    pub channel: Channel,
    pub released_at: Option<String>,
    /// In registry order, which is the preference order for the same OS and architecture.
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineInfo {
    pub eol: Option<CalDate>,
    pub security_only_since: Option<CalDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub id: String,
    pub display_name: String,
    pub license: Option<License>,
    pub lines: BTreeMap<String, LineInfo>,
    /// Newest first.
    pub releases: Vec<Release>,
}

/// An artifact chosen for this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactChoice<'a> {
    pub artifact: &'a Artifact,
    /// True when an Intel build was chosen for an Apple silicon Mac because no native one exists. It
    /// runs under Rosetta, and the UI should say so (docs/RUNTIMES.md section 3).
    pub rosetta_fallback: bool,
}

/// One row of the "available versions" list, as the UI needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AvailableVersion {
    pub runtime: String,
    pub version: String,
    /// The version line, for example `8.3` (the longest matching key of the runtime's `lines`).
    pub line: String,
    pub channel: Channel,
    /// The line has reached its end-of-life date.
    pub eol: bool,
    /// Download size in bytes for this computer, if there is a download for it.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// There is a download for this operating system and architecture.
    pub available: bool,
    /// The download is an Intel build that runs under Rosetta on this Apple silicon Mac.
    pub rosetta_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    runtimes: BTreeMap<String, Runtime>,
}

fn parse_date(text: &str, what: &str) -> Result<CalDate> {
    CalDate::parse(text)
        .ok_or_else(|| invalid(format!("{what} {text:?} is not a date like 2027-12-31")))
}

fn parse_artifact(runtime: &str, version: &str, raw: RawArtifact) -> Result<Option<Artifact>> {
    let os = match raw.os.as_str() {
        "windows" => Os::Windows,
        "macos" => Os::Macos,
        "linux" => Os::Linux,
        _ => return Ok(None), // an OS this build does not know: not usable here, not an error
    };
    let arch = match raw.arch.as_str() {
        "x86_64" => Arch::X86_64,
        "aarch64" => Arch::Aarch64,
        _ => return Ok(None),
    };
    let format = match raw.format.as_str() {
        "zip" => ArtifactFormat::Zip,
        "tar.gz" => ArtifactFormat::TarGz,
        "tar.xz" => ArtifactFormat::TarXz,
        "file" => ArtifactFormat::File,
        _ => return Ok(None),
    };

    let at = format!("{runtime} {version} ({}/{})", raw.os, raw.arch);
    let url = Url::parse(&raw.url)
        .map_err(|_| invalid(format!("{at}: {:?} is not a valid URL", raw.url)))?;
    if url.scheme() != "https" {
        return Err(invalid(format!(
            "{at}: the download URL must be https, got {:?}",
            raw.url
        )));
    }
    let sha = raw.sha256.trim();
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(format!(
            "{at}: sha256 must be 64 hexadecimal characters"
        )));
    }
    if raw.size == 0 {
        return Err(invalid(format!("{at}: size must be greater than zero")));
    }

    Ok(Some(Artifact {
        os,
        arch,
        variant: raw.variant,
        format,
        url: raw.url,
        sha256: sha.to_ascii_lowercase(),
        size: raw.size,
        strip_components: raw.strip_components,
        requires: raw.requires,
        extensions: raw.extensions,
    }))
}

fn parse_runtime(id: &str, raw: RawRuntime) -> Result<Runtime> {
    component(id).map_err(|_| invalid(format!("runtime id {id:?} is not a safe name")))?;

    let mut lines = BTreeMap::new();
    for (line, info) in raw.lines {
        let eol = info
            .eol
            .as_deref()
            .map(|d| parse_date(d, &format!("{id} line {line} eol")))
            .transpose()?;
        let security_only_since = info
            .security_only_since
            .as_deref()
            .map(|d| parse_date(d, &format!("{id} line {line} securityOnlySince")))
            .transpose()?;
        lines.insert(
            line,
            LineInfo {
                eol,
                security_only_since,
            },
        );
    }

    let mut releases: Vec<Release> = Vec::new();
    let mut seen_versions = HashSet::new();
    for raw_version in raw.versions {
        let version = RuntimeVersion::parse(&raw_version.version).ok_or_else(|| {
            invalid(format!(
                "{id}: {:?} is not a valid version",
                raw_version.version
            ))
        })?;
        component(version.as_str())
            .map_err(|_| invalid(format!("{id}: version {:?} is not a safe name", version)))?;
        if !seen_versions.insert(version.as_str().to_owned()) {
            return Err(invalid(format!("{id}: version {version} is listed twice")));
        }
        let channel = match raw_version.channel.as_deref() {
            None | Some("stable") => Channel::Stable,
            Some("rc") => Channel::Rc,
            Some("beta") => Channel::Beta,
            Some(_) => continue, // a channel this build does not know: do not offer it
        };

        let mut artifacts = Vec::new();
        let mut seen_targets = HashSet::new();
        for raw_artifact in raw_version.artifacts {
            if let Some(artifact) = parse_artifact(id, version.as_str(), raw_artifact)? {
                let target = (artifact.os, artifact.arch, artifact.variant.clone());
                let key = format!("{:?}/{:?}/{:?}", target.0, target.1, target.2);
                if !seen_targets.insert(key) {
                    return Err(invalid(format!(
                        "{id} {version}: two artifacts for the same {:?}/{:?}{}",
                        artifact.os,
                        artifact.arch,
                        artifact
                            .variant
                            .as_deref()
                            .map_or(String::new(), |v| format!(" variant {v}"))
                    )));
                }
                artifacts.push(artifact);
            }
        }
        releases.push(Release {
            version,
            channel,
            released_at: raw_version.released_at,
            artifacts,
        });
    }
    releases.sort_by(|a, b| b.version.cmp(&a.version)); // newest first, numerically

    Ok(Runtime {
        id: id.to_owned(),
        display_name: raw.display_name,
        license: raw.license,
        lines,
        releases,
    })
}

impl Registry {
    /// Parse and validate a registry document.
    pub fn parse(json_text: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(json_text)
            .map_err(|err| invalid(format!("the registry is not valid JSON: {err}")))?;

        // Look at `schema` first: a newer registry may have a different shape, and the useful answer
        // is "update lunaRun", not a confusing parse error.
        let schema = value
            .get("schema")
            .and_then(Value::as_u64)
            .filter(|schema| *schema >= 1)
            .ok_or_else(|| invalid("the registry has no valid \"schema\" number"))?;
        if schema > u64::from(SUPPORTED_SCHEMA) {
            return Err(LunaError::new(
                ErrorCode::RegistryUnsupportedSchema,
                format!("the registry uses schema {schema}, this version of lunaRun understands up to {SUPPORTED_SCHEMA}"),
            )
            .with_details(json!({ "found": schema, "supported": SUPPORTED_SCHEMA })));
        }

        let raw: RawRegistry = serde_json::from_value(value)
            .map_err(|err| invalid(format!("the registry has an unexpected structure: {err}")))?;
        let mut runtimes = BTreeMap::new();
        for (id, raw_runtime) in raw.runtimes {
            runtimes.insert(id.clone(), parse_runtime(&id, raw_runtime)?);
        }
        Ok(Self { runtimes })
    }

    pub fn runtime_ids(&self) -> impl Iterator<Item = &str> {
        self.runtimes.keys().map(String::as_str)
    }

    pub fn runtime(&self, id: &str) -> Option<&Runtime> {
        self.runtimes.get(id)
    }

    /// The download for `version` on this OS and architecture.
    ///
    /// On an Apple silicon Mac the native build is preferred, and an Intel build is used only if there
    /// is no native one (`rosetta_fallback`). Other platforms never fall back. If the registry lists
    /// several builds (variants) for the target, the first is used: the registry orders them by
    /// preference.
    pub fn artifact_for(
        &self,
        runtime: &str,
        version: &str,
        os: Os,
        arch: Arch,
    ) -> Result<ArtifactChoice<'_>> {
        let release = self
            .runtime(runtime)
            .and_then(|r| {
                r.releases
                    .iter()
                    .find(|release| release.version.as_str() == version)
            })
            .ok_or_else(|| {
                LunaError::new(
                    ErrorCode::NotFound,
                    format!("the registry has no {runtime} {version}"),
                )
            })?;
        release.select(os, arch).ok_or_else(|| {
            LunaError::new(
                ErrorCode::NoArtifactForPlatform,
                format!(
                    "{runtime} {version} is not available for this computer ({os:?}, {arch:?})"
                ),
            )
            .with_details(json!({ "runtime": runtime, "version": version }))
        })
    }

    /// Every release of every runtime as UI rows: runtimes in name order, newest version first.
    /// `today` is a parameter so end-of-life flags are deterministic in tests.
    pub fn available_versions(&self, os: Os, arch: Arch, today: CalDate) -> Vec<AvailableVersion> {
        self.runtimes
            .values()
            .flat_map(|runtime| {
                runtime.releases.iter().map(move |release| {
                    let choice = release.select(os, arch);
                    AvailableVersion {
                        runtime: runtime.id.clone(),
                        version: release.version.to_string(),
                        line: runtime.line_of(&release.version),
                        channel: release.channel,
                        eol: runtime.is_eol(&release.version, today),
                        size: choice.as_ref().map(|c| c.artifact.size),
                        available: choice.is_some(),
                        rosetta_fallback: choice.is_some_and(|c| c.rosetta_fallback),
                    }
                })
            })
            .collect()
    }
}

impl Release {
    fn select(&self, os: Os, arch: Arch) -> Option<ArtifactChoice<'_>> {
        let find = |wanted: Arch| {
            self.artifacts
                .iter()
                .find(|a| a.os == os && a.arch == wanted)
        };
        if let Some(artifact) = find(arch) {
            return Some(ArtifactChoice {
                artifact,
                rosetta_fallback: false,
            });
        }
        if os == Os::Macos
            && arch == Arch::Aarch64
            && let Some(artifact) = find(Arch::X86_64)
        {
            return Some(ArtifactChoice {
                artifact,
                rosetta_fallback: true,
            });
        }
        None
    }
}

impl Runtime {
    /// The line a version belongs to: the longest key of `lines` that is a prefix of the version
    /// (`8.3` over `8`), or the first two numbers when the registry lists no line for it.
    pub fn line_of(&self, version: &RuntimeVersion) -> String {
        self.lines
            .keys()
            .filter(|line| version.is_on_line(line))
            .max_by_key(|line| line.split('.').count())
            .cloned()
            .unwrap_or_else(|| version.major_minor())
    }

    /// Has the line of `version` reached its end-of-life date as of `today`? A line with no `eol`
    /// date (or an unknown line) is not end of life. The date itself counts as end of life.
    pub fn is_eol(&self, version: &RuntimeVersion, today: CalDate) -> bool {
        self.lines
            .get(&self.line_of(version))
            .and_then(|info| info.eol)
            .is_some_and(|eol| today >= eol)
    }
}
