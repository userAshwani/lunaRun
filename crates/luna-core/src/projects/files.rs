//! Reading the files of a project: read-only, size-limited, tolerant, and hashed for change detection.
//!
//! Nothing here writes. A file that is missing is nothing; one that is too large, unreadable or not JSON is a
//! [`Warning`] and detection goes on without it (docs/PROJECT-DETECTION.md section 1).

use std::fs;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Warning, WarningCode};
use crate::download::to_hex;

/// `composer.lock` of a big project is a few megabytes; more than this is not a lock file anyone wrote.
pub(super) const MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;
/// `.nvmrc`, `.php-version`, `.env`: a few lines.
pub(super) const MAX_TEXT_BYTES: u64 = 1024 * 1024;

pub(super) fn warning(
    code: WarningCode,
    file: Option<&str>,
    message: impl Into<String>,
    details: Vec<String>,
) -> Warning {
    Warning {
        code,
        file: file.map(str::to_owned),
        message: message.into(),
        details,
    }
}

/// The input files read so far: their warnings, and the hash of everything that was looked at.
pub(super) struct Inputs<'a> {
    dir: &'a Path,
    hasher: Sha256,
    pub warnings: Vec<Warning>,
}

impl<'a> Inputs<'a> {
    pub fn new(dir: &'a Path) -> Self {
        Self {
            dir,
            hasher: Sha256::new(),
            warnings: Vec::new(),
        }
    }

    fn feed(&mut self, label: &str, bytes: &[u8]) {
        self.hasher.update(label.as_bytes());
        self.hasher.update([0]);
        self.hasher.update((bytes.len() as u64).to_le_bytes());
        self.hasher.update(bytes);
        self.hasher.update([0]);
    }

    /// The bytes of `name` (relative to the project folder) if it is a readable file of at most `max` bytes. The hash
    /// takes in what was read, so a change to the file changes the hash.
    pub fn read(&mut self, name: &str, max: u64) -> Option<Vec<u8>> {
        let path = self.dir.join(name);
        let Ok(meta) = fs::metadata(&path) else {
            return None;
        };
        if !meta.is_file() {
            return None;
        }
        if meta.len() > max {
            self.feed(&format!("toolarge:{name}"), &meta.len().to_le_bytes());
            self.warnings.push(warning(
                WarningCode::FileTooLarge,
                Some(name),
                format!(
                    "{name} is {} bytes, more than the {max} that are read: it is ignored",
                    meta.len()
                ),
                Vec::new(),
            ));
            return None;
        }
        match fs::read(&path) {
            Ok(bytes) => {
                self.feed(name, &bytes);
                Some(bytes)
            }
            Err(error) => {
                self.feed(&format!("unreadable:{name}"), &[]);
                self.warnings.push(warning(
                    WarningCode::UnreadableFile,
                    Some(name),
                    format!("{name} cannot be read: {error}"),
                    Vec::new(),
                ));
                None
            }
        }
    }

    /// A JSON file as a value, or `None` (a missing file is nothing, a broken one is a warning).
    pub fn json(&mut self, name: &str) -> Option<Value> {
        let bytes = self.read(name, MAX_JSON_BYTES)?;
        let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        match serde_json::from_slice::<Value>(body) {
            Ok(value) => Some(value),
            Err(error) => {
                self.warnings.push(warning(
                    WarningCode::InvalidJson,
                    Some(name),
                    format!("{name} is not valid JSON ({error}): it is ignored"),
                    vec![format!("line {}, column {}", error.line(), error.column())],
                ));
                None
            }
        }
    }

    /// A small text file, decoded leniently (a byte order mark is dropped).
    pub fn text(&mut self, name: &str) -> Option<String> {
        let bytes = self.read(name, MAX_TEXT_BYTES)?;
        let text = String::from_utf8_lossy(&bytes);
        Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
    }

    /// Does `name` exist as a file (or, with `folder`, a folder)? The answer goes into the hash.
    pub fn exists(&mut self, name: &str, folder: bool) -> bool {
        let found = fs::metadata(self.dir.join(name))
            .is_ok_and(|m| if folder { m.is_dir() } else { m.is_file() });
        if found {
            self.feed(&format!("exists:{name}"), &[]);
        }
        found
    }

    pub fn finish(self) -> (String, Vec<Warning>) {
        (to_hex(&self.hasher.finalize()), self.warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn dir_with(files: &[(&str, &[u8])]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, bytes) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        dir
    }

    fn hash(dir: &Path, names: &[&str]) -> String {
        let mut inputs = Inputs::new(dir);
        for name in names {
            let _ = inputs.read(name, 1024);
        }
        inputs.finish().0
    }

    #[test]
    fn a_missing_file_is_nothing_and_a_folder_is_not_a_file() {
        let dir = dir_with(&[("sub/x.json", b"{}")]);
        let mut inputs = Inputs::new(dir.path());
        assert!(inputs.json("composer.json").is_none());
        assert!(inputs.text(".nvmrc").is_none());
        assert!(inputs.read("sub", 1024).is_none(), "a folder is not a file");
        assert!(
            inputs.warnings.is_empty(),
            "none of that is worth a warning"
        );
    }

    #[test]
    fn broken_json_is_a_warning_with_a_position_and_a_bom_is_fine() {
        let dir = dir_with(&[
            ("a.json", b"{ \"x\": }"),
            ("b.json", b"\xEF\xBB\xBF{\"x\":1}"),
        ]);
        let mut inputs = Inputs::new(dir.path());
        assert!(inputs.json("a.json").is_none());
        assert_eq!(inputs.json("b.json").unwrap()["x"], 1);
        assert_eq!(inputs.warnings.len(), 1);
        assert_eq!(inputs.warnings[0].code, WarningCode::InvalidJson);
        assert_eq!(inputs.warnings[0].file.as_deref(), Some("a.json"));
        assert!(inputs.warnings[0].details[0].starts_with("line 1"));
    }

    #[test]
    fn a_file_over_the_limit_is_left_unread_with_a_warning() {
        let dir = dir_with(&[("big.json", &[b' '; 100])]);
        let mut inputs = Inputs::new(dir.path());
        assert!(inputs.read("big.json", 50).is_none());
        assert_eq!(inputs.warnings[0].code, WarningCode::FileTooLarge);
        assert!(inputs.read("big.json", 500).is_some());
    }

    #[test]
    fn text_drops_a_byte_order_mark_and_survives_bad_utf8() {
        let dir = dir_with(&[(".nvmrc", b"\xEF\xBB\xBF20.11\n"), ("odd", b"caf\xE9")]);
        let mut inputs = Inputs::new(dir.path());
        assert_eq!(inputs.text(".nvmrc").as_deref(), Some("20.11\n"));
        assert!(inputs.text("odd").unwrap().starts_with("caf"));
    }

    #[test]
    fn the_hash_follows_the_inputs_and_nothing_else() {
        let first = dir_with(&[("a", b"one"), ("b", b"two")]);
        let same = dir_with(&[("a", b"one"), ("b", b"two")]);
        let changed = dir_with(&[("a", b"one"), ("b", b"twp")]);
        let fewer = dir_with(&[("a", b"one")]);
        let names = ["a", "b"];
        assert_eq!(
            hash(first.path(), &names),
            hash(same.path(), &names),
            "same files, same hash"
        );
        assert_ne!(
            hash(first.path(), &names),
            hash(changed.path(), &names),
            "a changed byte"
        );
        assert_ne!(
            hash(first.path(), &names),
            hash(fewer.path(), &names),
            "a missing file"
        );
        assert_eq!(hash(first.path(), &names).len(), 64);
        // Moving a byte from one file to the next must not give the same hash.
        let shifted_a = dir_with(&[("a", b"on"), ("b", b"etwo")]);
        assert_ne!(hash(first.path(), &names), hash(shifted_a.path(), &names));
    }

    #[test]
    fn marker_files_count_in_the_hash() {
        let with = dir_with(&[("artisan", b"")]);
        let without = dir_with(&[]);
        let hash_of = |dir: &Path| {
            let mut inputs = Inputs::new(dir);
            let found = inputs.exists("artisan", false);
            (found, inputs.finish().0)
        };
        let (found_with, hash_with) = hash_of(with.path());
        let (found_without, hash_without) = hash_of(without.path());
        assert!(found_with && !found_without);
        assert_ne!(hash_with, hash_without);
    }
}
