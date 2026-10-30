//! LUNA_HOME: where lunaRun keeps everything it creates, and the only code allowed to delete files.
//!
//! Layout is documented in docs/ARCHITECTURE.md §4. Everything lunaRun writes lives under one root
//! (NFR-04), and [`LunaHome::safe_remove`] refuses to touch anything outside it.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde_json::json;

use crate::error::{ErrorCode, IoResultExt, LunaError, Result};

/// Environment variable that overrides the default location.
pub const HOME_ENV: &str = "LUNA_HOME";

/// Sub-directories created under the root.
const LAYOUT: [&str; 6] = ["runtimes", "config", "data", "logs", "run", "cache"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunaHome {
    root: PathBuf,
    from_env: bool,
}

impl LunaHome {
    /// Use `root` as LUNA_HOME. Does not touch the disk; see [`LunaHome::ensure_layout`].
    ///
    /// Rejects values that would make a mistake catastrophic: relative paths, a filesystem root
    /// (`C:\`, `/`) and the user's home directory itself.
    pub fn at(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let reject = |why: &str| {
            Err(LunaError::new(
                ErrorCode::InvalidPath,
                format!(
                    "{} is not a valid lunaRun data directory: {why}",
                    root.display()
                ),
            )
            .with_details(json!({ "path": root.display().to_string() })))
        };
        if !root.is_absolute() {
            return reject("it must be an absolute path");
        }
        if root.parent().is_none() {
            return reject("it must not be a filesystem root");
        }
        if BaseDirs::new().is_some_and(|dirs| dirs.home_dir() == root) {
            return reject("it must not be your home directory");
        }
        Ok(Self {
            root,
            from_env: false,
        })
    }

    /// `LUNA_HOME` if set and non-empty, otherwise the per-OS default.
    pub fn resolve() -> Result<Self> {
        Self::resolve_with(std::env::var_os(HOME_ENV), default_root())
    }

    /// Pure core of [`LunaHome::resolve`], separated so it can be tested without touching the environment.
    fn resolve_with(env_value: Option<OsString>, default: Option<PathBuf>) -> Result<Self> {
        match env_value.filter(|value| !value.is_empty()) {
            Some(value) => Self::at(PathBuf::from(value)).map(|home| Self {
                from_env: true,
                ..home
            }),
            None => match default {
                Some(root) => Self::at(root),
                None => Err(LunaError::internal(
                    "cannot determine the user data directory; set the LUNA_HOME environment variable",
                )),
            },
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the root came from the `LUNA_HOME` environment variable (shown in Diagnostics).
    pub fn from_env(&self) -> bool {
        self.from_env
    }

    /// Create the standard directory layout. Safe to call repeatedly.
    pub fn ensure_layout(&self) -> Result<()> {
        for dir in LAYOUT {
            let path = self.root.join(dir);
            fs::create_dir_all(&path).at_path(&path)?;
        }
        Ok(())
    }

    pub fn runtimes_dir(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Download staging area. Kept out of [`LAYOUT`] on purpose: it is created on first download.
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }

    /// `runtimes/<runtime>/<version>`. Both parts typically come from the registry, so they are
    /// validated as single path components and can never escape `runtimes/`.
    pub fn runtime_dir(&self, runtime: &str, version: &str) -> Result<PathBuf> {
        Ok(self
            .runtimes_dir()
            .join(component(runtime)?)
            .join(component(version)?))
    }

    /// Delete a file or directory tree, but only if it lives strictly inside LUNA_HOME.
    ///
    /// - A path that does not exist is not an error (cleanup is idempotent).
    /// - The root itself is never removed.
    /// - A symlink is removed as a link; its target is never followed or deleted.
    /// - `..` and symlinked parents are resolved before the containment check.
    ///
    /// There is an unavoidable gap between the check and the delete (TOCTOU). It is acceptable
    /// because only lunaRun writes inside LUNA_HOME.
    pub fn safe_remove(&self, path: &Path) -> Result<()> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err).at_path(path),
        };

        let absolute = std::path::absolute(path).at_path(path)?;
        let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) else {
            return Err(outside(path, "it has no file name"));
        };
        // Resolve the parent (not the entry itself), so a symlink entry is judged by where the
        // link lives rather than where it points.
        let target = fs::canonicalize(parent).at_path(parent)?.join(name);
        let root = fs::canonicalize(&self.root).map_err(|_| {
            outside(
                path,
                "the lunaRun data directory does not exist, nothing can be removed",
            )
        })?;

        if target == root || !target.starts_with(&root) {
            return Err(outside(path, "it is not inside the lunaRun data directory"));
        }

        let kind = meta.file_type();
        let result = if kind.is_symlink() {
            // Directory symlinks on Windows must be removed with remove_dir.
            fs::remove_file(&target).or_else(|_| fs::remove_dir(&target))
        } else if kind.is_dir() {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        };
        result.at_path(&target)
    }
}

/// Per-OS default root (docs/ARCHITECTURE.md §4).
pub fn default_root() -> Option<PathBuf> {
    // Linux convention is lowercase directories; Windows and macOS use the product name.
    let dir_name = if cfg!(target_os = "linux") {
        crate::PRODUCT_NAME.to_lowercase()
    } else {
        crate::PRODUCT_NAME.to_owned()
    };
    BaseDirs::new().map(|dirs| dirs.data_local_dir().join(dir_name))
}

fn outside(path: &Path, why: &str) -> LunaError {
    LunaError::new(
        ErrorCode::PathOutsideRoot,
        format!("refusing to remove {}: {why}", path.display()),
    )
    .with_details(json!({ "path": path.display().to_string() }))
}

/// Accept only a plain single file-name component (no separators, no `..`, no drive letters).
pub(crate) fn component(part: &str) -> Result<&str> {
    let bad =
        part.is_empty() || part == "." || part == ".." || part.contains(['/', '\\', ':', '\0']);
    if bad {
        return Err(LunaError::new(
            ErrorCode::InvalidPath,
            format!("{part:?} is not a valid path component"),
        )
        .with_details(json!({ "component": part })));
    }
    Ok(part)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A LUNA_HOME whose path has a space and non-ASCII characters (NFR-07).
    fn home_in(tmp: &TempDir) -> LunaHome {
        let root = tmp.path().join("lüna home");
        fs::create_dir_all(&root).unwrap();
        LunaHome::at(root).unwrap()
    }

    #[test]
    fn at_rejects_dangerous_roots() {
        assert_eq!(
            LunaHome::at("relative/dir").unwrap_err().code,
            ErrorCode::InvalidPath
        );
        assert_eq!(LunaHome::at("").unwrap_err().code, ErrorCode::InvalidPath);
        let fs_root = std::env::current_dir()
            .unwrap()
            .ancestors()
            .last()
            .unwrap()
            .to_path_buf();
        assert_eq!(
            LunaHome::at(fs_root).unwrap_err().code,
            ErrorCode::InvalidPath
        );
        if let Some(dirs) = BaseDirs::new() {
            assert_eq!(
                LunaHome::at(dirs.home_dir()).unwrap_err().code,
                ErrorCode::InvalidPath
            );
        }
    }

    #[test]
    fn env_value_wins_over_default_and_empty_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let from_env = tmp.path().join("from env");
        let fallback = tmp.path().join("fallback");

        let home =
            LunaHome::resolve_with(Some(from_env.clone().into()), Some(fallback.clone())).unwrap();
        assert_eq!(home.root(), from_env);
        assert!(home.from_env());

        let home = LunaHome::resolve_with(Some(OsString::new()), Some(fallback.clone())).unwrap();
        assert_eq!(home.root(), fallback);

        let home = LunaHome::resolve_with(None, Some(fallback.clone())).unwrap();
        assert_eq!(home.root(), fallback);
        assert!(!home.from_env());

        assert_eq!(
            LunaHome::resolve_with(None, None).unwrap_err().code,
            ErrorCode::Internal
        );
    }

    #[test]
    fn default_root_ends_with_product_dir() {
        let root = default_root().unwrap();
        let name = root.file_name().unwrap().to_string_lossy().to_lowercase();
        assert_eq!(name, "lunarun");
    }

    #[test]
    fn ensure_layout_creates_dirs_and_is_repeatable() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        home.ensure_layout().unwrap();
        home.ensure_layout().unwrap();
        for dir in [
            home.runtimes_dir(),
            home.config_dir(),
            home.data_dir(),
            home.logs_dir(),
            home.run_dir(),
            home.cache_dir(),
        ] {
            assert!(dir.is_dir(), "{} should exist", dir.display());
        }
        assert!(
            !home.downloads_dir().exists(),
            "downloads is created lazily"
        );
    }

    #[test]
    fn runtime_dir_builds_expected_path_and_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let dir = home.runtime_dir("php", "8.3.14").unwrap();
        assert_eq!(dir, home.runtimes_dir().join("php").join("8.3.14"));

        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:", "x\0y"] {
            assert_eq!(
                home.runtime_dir("php", bad).unwrap_err().code,
                ErrorCode::InvalidPath,
                "version {bad:?}"
            );
            assert_eq!(
                home.runtime_dir(bad, "1.0.0").unwrap_err().code,
                ErrorCode::InvalidPath,
                "runtime {bad:?}"
            );
        }
    }

    #[test]
    fn safe_remove_deletes_files_and_trees_inside_root() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let tree = home.runtimes_dir().join("php").join("8.3.14").join("ext");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("a.dll"), b"x").unwrap();
        let file = home.root().join("loose file ü.txt");
        fs::write(&file, b"x").unwrap();

        home.safe_remove(&file).unwrap();
        assert!(!file.exists());
        home.safe_remove(&home.runtimes_dir().join("php")).unwrap();
        assert!(!home.runtimes_dir().join("php").exists());
        assert!(
            home.runtimes_dir().exists(),
            "only the requested tree is removed"
        );
    }

    #[test]
    fn safe_remove_ignores_missing_paths() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        home.safe_remove(&home.root().join("does-not-exist"))
            .unwrap();
    }

    #[test]
    fn safe_remove_refuses_the_root_itself() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let err = home.safe_remove(home.root()).unwrap_err();
        assert_eq!(err.code, ErrorCode::PathOutsideRoot);
        assert!(home.root().exists());
    }

    #[test]
    fn safe_remove_refuses_paths_outside_root() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let outside_dir = tmp.path().join("precious");
        fs::create_dir_all(&outside_dir).unwrap();
        fs::write(outside_dir.join("keep.txt"), b"x").unwrap();

        let err = home.safe_remove(&outside_dir).unwrap_err();
        assert_eq!(err.code, ErrorCode::PathOutsideRoot);
        assert!(outside_dir.join("keep.txt").exists());
    }

    #[test]
    fn safe_remove_refuses_dot_dot_escapes() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let precious = tmp.path().join("precious.txt");
        fs::write(&precious, b"x").unwrap();

        // <root>/../precious.txt reaches a sibling of the root.
        let sneaky = home.root().join("..").join("precious.txt");
        assert_eq!(
            home.safe_remove(&sneaky).unwrap_err().code,
            ErrorCode::PathOutsideRoot
        );
        assert!(precious.exists());

        // A path that ends in `..` has no file name at all.
        let up = home.root().join("..");
        assert_eq!(
            home.safe_remove(&up).unwrap_err().code,
            ErrorCode::PathOutsideRoot
        );
    }

    #[test]
    fn safe_remove_refuses_sibling_with_same_prefix() {
        // "<tmp>/lüna home-evil" starts with the same characters as "<tmp>/lüna home" but is not inside it.
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let evil = tmp.path().join("lüna home-evil");
        fs::create_dir_all(&evil).unwrap();
        assert_eq!(
            home.safe_remove(&evil).unwrap_err().code,
            ErrorCode::PathOutsideRoot
        );
        assert!(evil.exists());
    }

    #[test]
    fn safe_remove_unlinks_symlinks_without_touching_the_target() {
        let tmp = TempDir::new().unwrap();
        let home = home_in(&tmp);
        let target = tmp.path().join("precious");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), b"x").unwrap();
        let link = home.root().join("link");

        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&target, &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&target, &link).is_ok();
        if !linked {
            // Windows without Developer Mode / admin cannot create symlinks; nothing to test.
            return;
        }

        home.safe_remove(&link).unwrap();
        assert!(fs::symlink_metadata(&link).is_err(), "the link is gone");
        assert!(target.join("keep.txt").exists(), "the target is untouched");
    }
}
