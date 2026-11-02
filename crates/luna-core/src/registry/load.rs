//! Getting a registry: the network first, then the saved copy, then the one built into the app
//! (docs/RUNTIMES.md section 3, D-009).
//!
//! A registry is only written to the cache **after** it has been parsed and validated, so a bad
//! download (an error page, a half-written file, a registry for a newer lunaRun) can never replace a
//! good saved copy. The bundled copy is the last resort, so loading never fails for network reasons:
//! the caller always gets a usable registry and, when it is not the freshest one, the reason why.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use reqwest::Url;

use super::Registry;
use crate::download::Downloader;
use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::LunaHome;
use crate::state::write_atomic;

/// Environment variable that replaces the registry address: an `https://` URL, or, for development, a
/// local file path or `file://` URL.
pub const REGISTRY_URL_ENV: &str = "LUNA_REGISTRY_URL";

/// Where the registry is published: the generated file in the main repository (D-018), which is public so
/// no authentication is needed. A separate registry repository stays possible, only this address changes.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/userAshwani/lunaRun/main/registry/registry.json";

/// A registry is a few hundred kilobytes at most. Anything bigger is not one.
pub const MAX_REGISTRY_BYTES: u64 = 8 * 1024 * 1024;

/// The generated registry itself, not a copy of it: the build embeds exactly the file `pnpm registry`
/// writes, so the fallback cannot drift from it or be edited by hand. A daily workflow refreshes that
/// file, so every build ships a recent list. Anything building this crate needs the repository checkout.
const BUNDLED: &str = include_str!("../../../../registry/registry.json");

/// Where the registry that was returned came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrySource {
    /// Just fetched (or read from the override file).
    Fresh,
    /// The copy saved by an earlier run, and how old it is if the file system says.
    Cache { age: Option<Duration> },
    /// The registry compiled into this build: nothing newer was reachable or usable.
    Bundled,
}

#[derive(Debug)]
pub struct LoadedRegistry {
    pub registry: Registry,
    pub source: RegistrySource,
    /// For `Cache` and `Bundled`: why the fresh registry could not be used (offline, HTTP error,
    /// invalid, or `REGISTRY_UNSUPPORTED_SCHEMA` meaning "update lunaRun"). For `Fresh`: set only when
    /// the new registry could not be saved. Otherwise `None`.
    pub problem: Option<LunaError>,
}

/// The configured address: `LUNA_REGISTRY_URL` if it is set and not empty, else the default.
pub fn registry_location() -> String {
    location_from(std::env::var(REGISTRY_URL_ENV).ok().as_deref())
}

/// Separate from the environment lookup so tests need not change the process environment.
fn location_from(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => DEFAULT_REGISTRY_URL.to_owned(),
    }
}

pub fn cache_path(home: &LunaHome) -> PathBuf {
    home.cache_dir().join("registry.json")
}

/// The registry built into this build (the last resort when there is no network and no saved copy).
pub fn bundled_registry() -> Result<Registry> {
    Registry::parse(BUNDLED).map_err(|err| {
        LunaError::internal(format!("the bundled registry is invalid: {}", err.message))
    })
}

/// Load the best registry available. `location` is normally [`registry_location()`].
///
/// This only fails if the bundled registry itself is broken, which a test prevents.
pub async fn load_registry(
    home: &LunaHome,
    downloader: &Downloader,
    location: &str,
) -> Result<LoadedRegistry> {
    let problem = match fetch(downloader, location).await {
        Ok(fetched) => {
            // A local override file is already on disk, caching it would only confuse later runs.
            let save_error = fetched
                .cacheable
                .then(|| write_atomic(&cache_path(home), fetched.text.as_bytes()).err())
                .flatten();
            return Ok(LoadedRegistry {
                registry: fetched.registry,
                source: RegistrySource::Fresh,
                problem: save_error,
            });
        }
        Err(error) => error,
    };

    if let Some((registry, age)) = read_cache(&cache_path(home)) {
        return Ok(LoadedRegistry {
            registry,
            source: RegistrySource::Cache { age },
            problem: Some(problem),
        });
    }
    Ok(LoadedRegistry {
        registry: bundled_registry()?,
        source: RegistrySource::Bundled,
        problem: Some(problem),
    })
}

struct Fetched {
    registry: Registry,
    text: String,
    cacheable: bool,
}

async fn fetch(downloader: &Downloader, location: &str) -> Result<Fetched> {
    let remote = location.starts_with("https://") || location.starts_with("http://");
    let bytes = if remote {
        downloader.fetch_bytes(location, MAX_REGISTRY_BYTES).await?
    } else {
        let path = local_path(location)?;
        let size = tokio::fs::metadata(&path).await.at_path(&path)?.len();
        if size > MAX_REGISTRY_BYTES {
            return Err(LunaError::new(
                ErrorCode::RegistryInvalid,
                format!("{} is larger than a registry can be", path.display()),
            ));
        }
        tokio::fs::read(&path).await.at_path(&path)?
    };
    let text = String::from_utf8(bytes).map_err(|_| {
        LunaError::new(
            ErrorCode::RegistryInvalid,
            "the registry is not valid UTF-8 text",
        )
    })?;
    let registry = Registry::parse(&text)?;
    Ok(Fetched {
        registry,
        text,
        cacheable: remote,
    })
}

/// A local override: a `file://` URL or a plain path. A plain path is not parsed as a URL, because
/// `C:\registry.json` would read as the scheme `c`.
fn local_path(location: &str) -> Result<PathBuf> {
    if location.starts_with("file://") {
        Url::parse(location)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or_else(|| {
                LunaError::new(
                    ErrorCode::InvalidUrl,
                    format!("{location:?} is not a usable file:// address"),
                )
            })
    } else {
        Ok(PathBuf::from(location))
    }
}

/// The saved registry, if there is one that is still valid for this version of lunaRun.
fn read_cache(path: &Path) -> Option<(Registry, Option<Duration>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let registry = Registry::parse(&text).ok()?;
    let age = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok());
    Some((registry, age))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_unless_it_is_missing_or_blank() {
        assert_eq!(location_from(None), DEFAULT_REGISTRY_URL);
        assert_eq!(location_from(Some("")), DEFAULT_REGISTRY_URL);
        assert_eq!(location_from(Some("   ")), DEFAULT_REGISTRY_URL);
        assert_eq!(
            location_from(Some(" https://example.com/r.json ")),
            "https://example.com/r.json"
        );
        assert_eq!(location_from(Some("D:\\dev\\r.json")), "D:\\dev\\r.json");
    }

    #[test]
    fn the_default_address_is_https() {
        assert!(DEFAULT_REGISTRY_URL.starts_with("https://"));
    }
}
