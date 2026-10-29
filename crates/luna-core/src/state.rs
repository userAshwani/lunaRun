//! Persistent state: small JSON files, written atomically, with schema versions and migrations.
//!
//! Rules (NFR-08, D-006): a write never leaves a half-written file; a corrupt file is moved aside
//! and replaced by defaults instead of crashing; a file from a *newer* lunaRun is never overwritten.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::{ErrorCode, IoResultExt, LunaError, Result};

/// A type stored as a JSON file. The `"schema"` field is added on save and handled on load, so the
/// type itself does not declare it.
pub trait Persisted: Serialize + DeserializeOwned + Default {
    /// Current schema version. Starts at 1.
    const SCHEMA_VERSION: u32;

    /// Upgrade the raw JSON from schema `from` to `from + 1`. Called repeatedly until current.
    /// The default fails, which is correct while only one version exists.
    fn migrate(from: u32, value: Value) -> Result<Value> {
        let _ = value;
        Err(LunaError::internal(format!(
            "no migration from schema version {from}"
        )))
    }
}

/// How a state file was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No file yet: defaults were returned and nothing was written.
    Fresh,
    /// Read as is.
    Loaded,
    /// Upgraded from an older schema and saved. The original is kept at `backup`.
    Migrated { from: u32, backup: PathBuf },
    /// The file was unreadable, so it was moved to `backup` and defaults were returned.
    Recovered { backup: PathBuf, reason: String },
}

#[derive(Debug)]
pub struct Loaded<T> {
    pub value: T,
    pub outcome: LoadOutcome,
}

/// Load `path`, applying the rules in the module docs.
pub fn load<T: Persisted>(path: &Path) -> Result<Loaded<T>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded {
                value: T::default(),
                outcome: LoadOutcome::Fresh,
            });
        }
        Err(err) => return Err(err).at_path(path),
    };

    let mut value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(err) => return recover(path, format!("invalid JSON: {err}")),
    };

    let from = match value
        .get("schema")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
    {
        Some(version) if version >= 1 => version,
        _ => return recover(path, "missing or invalid \"schema\" field".to_owned()),
    };

    if from > T::SCHEMA_VERSION {
        return Err(LunaError::new(
            ErrorCode::StateSchemaTooNew,
            format!(
                "{} was written by a newer version of lunaRun (schema {from}, this version supports up to {})",
                path.display(),
                T::SCHEMA_VERSION
            ),
        )
        .with_details(json!({
            "path": path.display().to_string(),
            "found": from,
            "supported": T::SCHEMA_VERSION,
        })));
    }

    let migrating = from < T::SCHEMA_VERSION;
    let backup = if migrating {
        let backup = unique_sibling(path, &format!("pre-migration-v{from}"));
        fs::copy(path, &backup).at_path(&backup)?;
        for version in from..T::SCHEMA_VERSION {
            value = T::migrate(version, value)?;
        }
        Some(backup)
    } else {
        None
    };

    match serde_json::from_value::<T>(value) {
        Ok(parsed) => match backup {
            Some(backup) => {
                save(path, &parsed)?;
                Ok(Loaded {
                    value: parsed,
                    outcome: LoadOutcome::Migrated { from, backup },
                })
            }
            None => Ok(Loaded {
                value: parsed,
                outcome: LoadOutcome::Loaded,
            }),
        },
        Err(err) => recover(path, format!("unexpected structure: {err}")),
    }
}

/// Serialize `value`, add the schema field and write it atomically.
pub fn save<T: Persisted>(path: &Path, value: &T) -> Result<()> {
    let mut json = serde_json::to_value(value)
        .map_err(|err| LunaError::internal(format!("cannot serialize state: {err}")))?;
    let Some(object) = json.as_object_mut() else {
        return Err(LunaError::internal(
            "persisted state must serialize to a JSON object",
        ));
    };
    object.insert("schema".to_owned(), T::SCHEMA_VERSION.into());
    let mut bytes = serde_json::to_vec_pretty(&json)
        .map_err(|err| LunaError::internal(format!("cannot serialize state: {err}")))?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

/// Write `bytes` to `path` so readers see either the old or the new content, never a mix:
/// temp file in the same directory, fsync, then rename over the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).at_path(dir)?;

    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = dir.join(name);

    let write = || -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    };
    match write() {
        Ok(()) => {
            sync_dir(dir);
            Ok(())
        }
        Err(err) => {
            // Best effort: do not leave the temp file behind.
            let _ = fs::remove_file(&tmp);
            Err(err).at_path(path)
        }
    }
}

/// Persist the rename itself. Not possible on Windows, where it is also not needed.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Move an unreadable file aside and return defaults.
fn recover<T: Persisted>(path: &Path, reason: String) -> Result<Loaded<T>> {
    let backup = unique_sibling(path, "corrupt");
    fs::rename(path, &backup).at_path(path)?;
    Ok(Loaded {
        value: T::default(),
        outcome: LoadOutcome::Recovered { backup, reason },
    })
}

/// `<file>.<label>-<unix millis>` next to `path`, with a counter if that name is taken.
fn unique_sibling(path: &Path, label: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let base = path.file_name().map(OsString::from).unwrap_or_default();
    let mut attempt = 0u32;
    loop {
        let mut name = base.clone();
        name.push(if attempt == 0 {
            format!(".{label}-{millis}")
        } else {
            format!(".{label}-{millis}-{attempt}")
        });
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::TempDir;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Prefs {
        name: String,
        count: u32,
    }

    impl Persisted for Prefs {
        const SCHEMA_VERSION: u32 = 1;
    }

    /// Schema 2 renamed `title` to `name`.
    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct PrefsV2 {
        name: String,
    }

    impl Persisted for PrefsV2 {
        const SCHEMA_VERSION: u32 = 2;

        fn migrate(from: u32, mut value: Value) -> Result<Value> {
            assert_eq!(from, 1);
            let object = value.as_object_mut().unwrap();
            let title = object.remove("title").unwrap_or(Value::Null);
            object.insert("name".to_owned(), title);
            object.insert("schema".to_owned(), 2.into());
            Ok(value)
        }
    }

    /// A directory whose name has a space and non-ASCII characters (NFR-07).
    fn dir() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("stäte dir");
        fs::create_dir_all(&dir).unwrap();
        (tmp, dir)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn missing_file_gives_defaults_and_writes_nothing() {
        let (_tmp, dir) = dir();
        let loaded = load::<Prefs>(&dir.join("prefs.json")).unwrap();
        assert_eq!(loaded.value, Prefs::default());
        assert_eq!(loaded.outcome, LoadOutcome::Fresh);
        assert!(files_in(&dir).is_empty());
    }

    #[test]
    fn save_then_load_round_trips_and_stores_the_schema() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        let prefs = Prefs {
            name: "Zoë".into(),
            count: 3,
        };
        save(&path, &prefs).unwrap();

        let raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["schema"], 1);

        let loaded = load::<Prefs>(&path).unwrap();
        assert_eq!(loaded.value, prefs);
        assert_eq!(loaded.outcome, LoadOutcome::Loaded);
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        save(
            &path,
            &Prefs {
                name: "one".into(),
                count: 1,
            },
        )
        .unwrap();
        save(
            &path,
            &Prefs {
                name: "two".into(),
                count: 2,
            },
        )
        .unwrap();
        assert_eq!(load::<Prefs>(&path).unwrap().value.name, "two");
        assert_eq!(files_in(&dir), vec!["prefs.json"]);
    }

    #[test]
    fn write_atomic_creates_missing_parent_directories() {
        let (_tmp, dir) = dir();
        let path = dir.join("a").join("b").join("file.txt");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn corrupt_json_is_moved_aside_and_defaults_returned() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        fs::write(&path, b"{ this is not json").unwrap();

        let loaded = load::<Prefs>(&path).unwrap();
        assert_eq!(loaded.value, Prefs::default());
        let LoadOutcome::Recovered { backup, reason } = loaded.outcome else {
            panic!("expected Recovered");
        };
        assert!(reason.contains("invalid JSON"));
        assert!(!path.exists(), "the corrupt file is no longer in the way");
        assert_eq!(fs::read(&backup).unwrap(), b"{ this is not json");
    }

    #[test]
    fn wrong_shape_and_missing_schema_are_treated_as_corrupt() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");

        fs::write(&path, br#"{ "name": "x" }"#).unwrap();
        let loaded = load::<Prefs>(&path).unwrap();
        assert!(matches!(loaded.outcome, LoadOutcome::Recovered { .. }));

        fs::write(&path, br#"{ "schema": 1, "name": 5, "count": "many" }"#).unwrap();
        let loaded = load::<Prefs>(&path).unwrap();
        assert!(matches!(loaded.outcome, LoadOutcome::Recovered { .. }));

        fs::write(&path, br#"{ "schema": 0, "name": "x", "count": 1 }"#).unwrap();
        assert!(matches!(
            load::<Prefs>(&path).unwrap().outcome,
            LoadOutcome::Recovered { .. }
        ));
    }

    #[test]
    fn repeated_corruption_never_overwrites_an_earlier_backup() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        for round in 0..3 {
            fs::write(&path, format!("garbage {round}")).unwrap();
            load::<Prefs>(&path).unwrap();
        }
        assert_eq!(
            files_in(&dir).len(),
            3,
            "three distinct backups: {:?}",
            files_in(&dir)
        );
    }

    #[test]
    fn newer_schema_is_an_error_and_the_file_is_untouched() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        let original = br#"{ "schema": 99, "name": "future", "count": 1 }"#;
        fs::write(&path, original).unwrap();

        let err = load::<Prefs>(&path).unwrap_err();
        assert_eq!(err.code, ErrorCode::StateSchemaTooNew);
        let details = err.details.unwrap();
        assert_eq!(details["found"], 99);
        assert_eq!(details["supported"], 1);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(files_in(&dir), vec!["prefs.json"]);
    }

    #[test]
    fn older_schema_is_migrated_saved_and_backed_up() {
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        let original = br#"{ "schema": 1, "title": "Old title" }"#;
        fs::write(&path, original).unwrap();

        let loaded = load::<PrefsV2>(&path).unwrap();
        assert_eq!(
            loaded.value,
            PrefsV2 {
                name: "Old title".into()
            }
        );
        let LoadOutcome::Migrated { from, backup } = loaded.outcome else {
            panic!("expected Migrated");
        };
        assert_eq!(from, 1);
        assert_eq!(fs::read(&backup).unwrap(), original);

        // The migrated file on disk is now schema 2 and loads without migrating again.
        let again = load::<PrefsV2>(&path).unwrap();
        assert_eq!(again.outcome, LoadOutcome::Loaded);
        assert_eq!(again.value.name, "Old title");
    }

    #[test]
    fn failing_migration_keeps_the_original_file() {
        // `Prefs` has no migration, so asking it to read an older schema must fail cleanly.
        #[derive(Debug, Default, Serialize, Deserialize)]
        struct Needs3 {}
        impl Persisted for Needs3 {
            const SCHEMA_VERSION: u32 = 3;
        }
        let (_tmp, dir) = dir();
        let path = dir.join("prefs.json");
        let original = br#"{ "schema": 1 }"#;
        fs::write(&path, original).unwrap();

        let err = load::<Needs3>(&path).unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn non_object_state_cannot_be_saved() {
        #[derive(Debug, Default, Serialize, Deserialize)]
        struct Bare(u32);
        impl Persisted for Bare {
            const SCHEMA_VERSION: u32 = 1;
        }
        let (_tmp, dir) = dir();
        let err = save(&dir.join("bare.json"), &Bare(1)).unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
    }
}
