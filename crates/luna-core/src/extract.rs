//! Safe extraction of `.zip`, `.tar.gz` and `.tar.xz` archives (FR-RT-02, NFR-05, golden rule 5).
//!
//! Archives come from the internet, so every archive is treated as hostile. Nothing is ever written
//! outside the destination directory, whatever the archive says. The rules:
//!
//! - **Paths** are checked before anything is created: no absolute paths, drive letters, UNC paths,
//!   `..` components, NUL bytes (and on Windows no reserved device names, `:` or other forbidden
//!   characters). Both `/` and `\` count as separators, so `..\evil` is caught on every platform.
//! - **The destination must be empty**, so lunaRun itself created everything that is in it. That lets
//!   us track every symlink we make and refuse to write *through* one. Writing through a symlink is
//!   the classic way out of the destination, even when the link itself points inside it.
//! - **Symlinks** (Unix only) must have a relative target that stays inside the destination. On
//!   Windows they are refused. **Hard links** must point at a regular file we extracted earlier.
//!   Devices, FIFOs and other special files are refused.
//! - **Files are created with `create_new`**, which never follows an existing link and never
//!   overwrites, so duplicate names are refused too.
//! - **Limits** on total size, size per entry and entry count stop decompression bombs. Sizes are
//!   counted from the bytes actually produced, never from what the archive claims. XZ decoder memory
//!   is capped as well.
//! - **Permissions:** only the `rwx` bits are kept. setuid, setgid and the sticky bit are dropped, and
//!   ownership is ignored.
//!
//! Extraction is not atomic: on any error the destination may hold part of the archive. The caller
//! extracts into a fresh staging directory and deletes it with `LunaHome::safe_remove` on failure.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::error::{ErrorCode, IoResultExt, LunaError, Result};

const COPY_BUFFER: usize = 64 * 1024;
const FILE_BUFFER: usize = 256 * 1024;
const MAX_SYMLINK_TARGET: u64 = 4096;

/// Supported archive formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    TarXz,
}

impl ArchiveFormat {
    /// From a file name such as `php-8.3.14.zip` or `node-v24.tar.xz`.
    pub fn from_file_name(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        if name.ends_with(".zip") {
            Some(Self::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
            Some(Self::TarXz)
        } else {
            None
        }
    }

    /// From the `format` field of a registry artifact (docs/RUNTIMES.md section 3).
    pub fn from_registry(format: &str) -> Option<Self> {
        match format {
            "zip" => Some(Self::Zip),
            "tar.gz" => Some(Self::TarGz),
            "tar.xz" => Some(Self::TarXz),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// Leading path components to drop from every entry (a `node-v24/` wrapper folder, say). Entries
    /// that have no components left are skipped.
    pub strip_components: usize,
    /// Abort when more bytes than this would be written in total.
    pub max_total_bytes: u64,
    /// Abort when one file is larger than this.
    pub max_entry_bytes: u64,
    /// Abort when the archive has more entries than this.
    pub max_entries: u64,
    /// Memory the XZ decoder may use, in KiB. Official `xz -9` needs 64 MiB.
    pub xz_memory_limit_kib: u32,
}

impl Default for ExtractOptions {
    /// Generous for real runtimes (MySQL is about 1 GB unpacked) but far below a decompression bomb.
    fn default() -> Self {
        Self {
            strip_components: 0,
            max_total_bytes: 8 * 1024 * 1024 * 1024,
            max_entry_bytes: 4 * 1024 * 1024 * 1024,
            max_entries: 250_000,
            xz_memory_limit_kib: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractProgress {
    /// Bytes written to disk so far.
    pub bytes_written: u64,
    /// Total uncompressed size, known up front for `.zip` only.
    pub bytes_total: Option<u64>,
    pub entries_done: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExtractSummary {
    pub files: u64,
    pub directories: u64,
    pub symlinks: u64,
    pub hard_links: u64,
    pub bytes: u64,
}

fn unsafe_entry(entry: &str, why: impl std::fmt::Display) -> LunaError {
    LunaError::new(
        ErrorCode::ArchiveUnsafe,
        format!("unsafe archive entry {entry:?}: {why}"),
    )
    .with_details(json!({ "entry": entry }))
}

fn corrupt(why: impl std::fmt::Display) -> LunaError {
    LunaError::new(
        ErrorCode::ArchiveCorrupt,
        format!("the archive is damaged or unsupported: {why}"),
    )
}

fn too_large(why: impl std::fmt::Display) -> LunaError {
    LunaError::new(
        ErrorCode::ArchiveTooLarge,
        format!("the archive is too large: {why}"),
    )
}

fn cancelled() -> LunaError {
    LunaError::new(ErrorCode::Cancelled, "the extraction was cancelled")
}

// ---------------------------------------------------------------------------------------------
// Path rules (pure functions, unit tested below)
// ---------------------------------------------------------------------------------------------

fn is_windows_reserved(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

fn has_drive_letter(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Validate an archive entry name and remove `strip` leading components.
///
/// `Ok(None)` means the entry is the stripped wrapper itself and is skipped. `Err` carries the reason
/// the name is unsafe. `windows` selects the extra rules that only matter on Windows file systems.
pub(crate) fn sanitize_entry_path(
    raw: &str,
    strip: usize,
    windows: bool,
) -> std::result::Result<Option<PathBuf>, String> {
    if raw.contains('\0') {
        return Err("the name contains a NUL byte".to_owned());
    }
    let unified = raw.replace('\\', "/");
    if unified.starts_with('/') {
        return Err("absolute paths are not allowed".to_owned());
    }
    if has_drive_letter(&unified) {
        return Err("paths with a drive letter are not allowed".to_owned());
    }

    let mut components: Vec<&str> = Vec::new();
    for component in unified.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err("a parent directory (..) component is not allowed".to_owned()),
            name => {
                if windows {
                    if name
                        .chars()
                        .any(|c| c.is_control() || "<>:\"|?*".contains(c))
                    {
                        return Err(format!("{name:?} has characters Windows does not allow"));
                    }
                    if name.ends_with('.') || name.ends_with(' ') {
                        return Err(format!(
                            "{name:?} ends in a dot or space, which Windows alters"
                        ));
                    }
                    if is_windows_reserved(name) {
                        return Err(format!("{name:?} is a reserved Windows device name"));
                    }
                }
                components.push(name);
            }
        }
    }

    if components.len() <= strip {
        return Ok(None);
    }
    Ok(Some(components[strip..].iter().collect()))
}

/// Does a relative symlink at `link` (relative to the destination) with this `target` stay inside?
pub(crate) fn symlink_target_stays_inside(
    link: &Path,
    target: &str,
) -> std::result::Result<(), String> {
    if target.is_empty() {
        return Err("the link target is empty".to_owned());
    }
    if target.contains('\0') {
        return Err("the link target contains a NUL byte".to_owned());
    }
    let unified = target.replace('\\', "/");
    if unified.starts_with('/') || has_drive_letter(&unified) {
        return Err("absolute link targets are not allowed".to_owned());
    }

    // How many directories deep the link's own folder is. `..` may not go above the destination.
    let mut depth = link.components().count().saturating_sub(1);
    for component in unified.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if depth == 0 {
                    return Err("the link points outside the destination".to_owned());
                }
                depth -= 1;
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

/// Case-insensitive key, so `Link` and `link/x` collide on case-insensitive file systems too.
fn link_key(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------------------------
// Extraction state shared by every format
// ---------------------------------------------------------------------------------------------

struct Extraction<'a> {
    dest: &'a Path,
    options: &'a ExtractOptions,
    cancel: &'a CancellationToken,
    on_progress: &'a mut dyn FnMut(ExtractProgress),
    bytes_total: Option<u64>,
    summary: ExtractSummary,
    entries_seen: u64,
    /// Keys of the symbolic links created so far.
    symlinks: HashSet<String>,
    /// Relative paths of the regular files created so far (the only valid hard-link targets).
    files: HashSet<PathBuf>,
}

impl<'a> Extraction<'a> {
    fn new(
        dest: &'a Path,
        options: &'a ExtractOptions,
        cancel: &'a CancellationToken,
        on_progress: &'a mut dyn FnMut(ExtractProgress),
        bytes_total: Option<u64>,
    ) -> Self {
        Self {
            dest,
            options,
            cancel,
            on_progress,
            bytes_total,
            summary: ExtractSummary::default(),
            entries_seen: 0,
            symlinks: HashSet::new(),
            files: HashSet::new(),
        }
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    fn begin_entry(&mut self) -> Result<()> {
        self.check_cancel()?;
        self.entries_seen += 1;
        if self.entries_seen > self.options.max_entries {
            return Err(too_large(format!(
                "it has more than {} entries",
                self.options.max_entries
            )));
        }
        Ok(())
    }

    fn report(&mut self) {
        let progress = ExtractProgress {
            bytes_written: self.summary.bytes,
            bytes_total: self.bytes_total,
            entries_done: self.entries_seen,
        };
        (self.on_progress)(progress);
    }

    /// Check a raw entry name and return its place inside the destination, or `None` to skip it.
    fn resolve(&self, raw: &str) -> Result<Option<PathBuf>> {
        sanitize_entry_path(raw, self.options.strip_components, cfg!(windows))
            .map_err(|why| unsafe_entry(raw, why))
    }

    /// Refuse to create `rel` if it, or any folder above it, is a symlink we made.
    fn check_not_through_symlink(&self, rel: &Path, raw: &str) -> Result<()> {
        let mut prefix = PathBuf::new();
        for component in rel.components() {
            prefix.push(component);
            if self.symlinks.contains(&link_key(&prefix)) {
                return Err(unsafe_entry(
                    raw,
                    format!(
                        "it would be written through the symbolic link {}",
                        prefix.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn prepare_parent(&self, target: &Path, raw: &str) -> Result<()> {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists || is_not_a_directory(&err) {
                    unsafe_entry(raw, "a folder it needs is already used by a file")
                } else {
                    LunaError::from(err)
                }
            })?;
        }
        Ok(())
    }

    fn make_directory(&mut self, rel: &Path, mode: Option<u32>, raw: &str) -> Result<()> {
        self.check_not_through_symlink(rel, raw)?;
        let target = self.dest.join(rel);
        fs::create_dir_all(&target).map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists || is_not_a_directory(&err) {
                unsafe_entry(raw, "it conflicts with an existing file")
            } else {
                LunaError::from(err)
            }
        })?;
        set_mode(&target, mode, true)?;
        self.summary.directories += 1;
        Ok(())
    }

    fn write_file(
        &mut self,
        rel: &Path,
        reader: &mut dyn Read,
        mode: Option<u32>,
        raw: &str,
    ) -> Result<()> {
        self.check_not_through_symlink(rel, raw)?;
        let target = self.dest.join(rel);
        self.prepare_parent(&target, raw)?;

        // `create_new` fails on anything that already exists, including a symlink, and never follows one.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    unsafe_entry(raw, "an entry with this name was already extracted")
                } else {
                    LunaError::from(err)
                }
            })?;

        let mut buffer = vec![0u8; COPY_BUFFER];
        let mut entry_bytes: u64 = 0;
        loop {
            self.check_cancel()?;
            let read = reader.read(&mut buffer).map_err(corrupt)?;
            if read == 0 {
                break;
            }
            entry_bytes += read as u64;
            self.summary.bytes += read as u64;
            if entry_bytes > self.options.max_entry_bytes {
                return Err(too_large(format!(
                    "{raw:?} is larger than {} bytes",
                    self.options.max_entry_bytes
                )));
            }
            if self.summary.bytes > self.options.max_total_bytes {
                return Err(too_large(format!(
                    "it expands to more than {} bytes",
                    self.options.max_total_bytes
                )));
            }
            file.write_all(&buffer[..read]).at_path(&target)?;
            self.report();
        }
        drop(file);

        set_mode(&target, mode, false)?;
        self.files.insert(rel.to_path_buf());
        self.summary.files += 1;
        self.report();
        Ok(())
    }

    fn make_symlink(&mut self, rel: &Path, link_target: &str, raw: &str) -> Result<()> {
        self.check_not_through_symlink(rel, raw)?;
        symlink_target_stays_inside(rel, link_target).map_err(|why| unsafe_entry(raw, why))?;
        let path = self.dest.join(rel);
        self.prepare_parent(&path, raw)?;
        create_symlink(link_target, &path, raw)?;
        self.symlinks.insert(link_key(rel));
        self.summary.symlinks += 1;
        Ok(())
    }

    fn make_hard_link(&mut self, rel: &Path, target_raw: &str, raw: &str) -> Result<()> {
        self.check_not_through_symlink(rel, raw)?;
        let target_rel = self.resolve(target_raw)?.ok_or_else(|| {
            unsafe_entry(raw, "its hard link target is outside the archive contents")
        })?;
        if !self.files.contains(&target_rel) {
            return Err(unsafe_entry(
                raw,
                "a hard link must point at a regular file extracted earlier",
            ));
        }
        let path = self.dest.join(rel);
        self.prepare_parent(&path, raw)?;
        fs::hard_link(self.dest.join(&target_rel), &path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                unsafe_entry(raw, "an entry with this name was already extracted")
            } else {
                LunaError::from(err)
            }
        })?;
        self.files.insert(rel.to_path_buf());
        self.summary.hard_links += 1;
        Ok(())
    }
}

fn is_not_a_directory(err: &std::io::Error) -> bool {
    // `ErrorKind::NotADirectory` is not stable on every toolchain we build with, so compare the message
    // of the two platform codes (Unix ENOTDIR = 20, Windows ERROR_DIRECTORY = 267).
    matches!(err.raw_os_error(), Some(20 | 267))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>, is_dir: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // Keep only rwx. Drop setuid, setgid and the sticky bit. Always leave the owner able to use it.
    let default = if is_dir { 0o755 } else { 0o644 };
    let mut bits = mode.map_or(default, |m| m & 0o777);
    bits |= if is_dir { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).at_path(path)
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>, _is_dir: bool) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &str, link: &Path, raw: &str) -> Result<()> {
    std::os::unix::fs::symlink(target, link).map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            unsafe_entry(raw, "an entry with this name was already extracted")
        } else {
            LunaError::from(err)
        }
    })
}

#[cfg(not(unix))]
fn create_symlink(_target: &str, _link: &Path, raw: &str) -> Result<()> {
    Err(unsafe_entry(
        raw,
        "symbolic links are not supported when extracting on Windows",
    ))
}

fn prepare_destination(dest: &Path) -> Result<()> {
    if dest.exists() {
        let mut entries = fs::read_dir(dest).map_err(|_| {
            LunaError::new(
                ErrorCode::InvalidPath,
                format!("{} is not a folder", dest.display()),
            )
        })?;
        if entries.next().is_some() {
            return Err(LunaError::new(
                ErrorCode::InvalidPath,
                format!("{} must be an empty folder to extract into", dest.display()),
            ));
        }
        Ok(())
    } else {
        fs::create_dir_all(dest).at_path(dest)
    }
}

// ---------------------------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------------------------

fn extract_zip(
    archive_path: &Path,
    ctx_dest: &Path,
    options: &ExtractOptions,
    cancel: &CancellationToken,
    on_progress: &mut dyn FnMut(ExtractProgress),
) -> Result<ExtractSummary> {
    let file = File::open(archive_path).at_path(archive_path)?;
    let mut archive =
        zip::ZipArchive::new(BufReader::with_capacity(FILE_BUFFER, file)).map_err(corrupt)?;

    if archive.is_empty() {
        return Err(corrupt("it contains no files"));
    }

    // Declared sizes give a progress bar. They are only a hint, the limits use real byte counts.
    let mut declared: u64 = 0;
    for index in 0..archive.len() {
        declared = declared.saturating_add(archive.by_index_raw(index).map_err(corrupt)?.size());
    }
    let mut ctx = Extraction::new(ctx_dest, options, cancel, on_progress, Some(declared));

    for index in 0..archive.len() {
        ctx.begin_entry()?;
        let mut entry = archive.by_index(index).map_err(corrupt)?;
        // Zip names are not guaranteed to be UTF-8. Decode what we can, and fall back to the raw bytes.
        let raw = match entry.name() {
            Ok(name) => name.into_owned(),
            Err(_) => String::from_utf8_lossy(entry.name_raw()).into_owned(),
        };
        let unix_mode = entry.unix_mode();
        let is_symlink = unix_mode.is_some_and(|mode| mode & 0o170_000 == 0o120_000);

        let Some(rel) = ctx.resolve(&raw)? else {
            continue;
        };
        if entry.is_dir() {
            ctx.make_directory(&rel, unix_mode, &raw)?;
        } else if is_symlink {
            let mut target = String::new();
            entry
                .by_ref()
                .take(MAX_SYMLINK_TARGET + 1)
                .read_to_string(&mut target)
                .map_err(corrupt)?;
            if target.len() as u64 > MAX_SYMLINK_TARGET {
                return Err(unsafe_entry(&raw, "the link target is unreasonably long"));
            }
            ctx.make_symlink(&rel, &target, &raw)?;
        } else {
            ctx.write_file(&rel, &mut entry, unix_mode, &raw)?;
        }
        ctx.report();
    }
    Ok(ctx.summary)
}

fn extract_tar(
    reader: impl Read,
    dest: &Path,
    options: &ExtractOptions,
    cancel: &CancellationToken,
    on_progress: &mut dyn FnMut(ExtractProgress),
) -> Result<ExtractSummary> {
    let mut ctx = Extraction::new(dest, options, cancel, on_progress, None);
    let mut archive = tar::Archive::new(reader);

    for entry in archive.entries().map_err(corrupt)? {
        let mut entry = entry.map_err(corrupt)?;
        let kind = entry.header().entry_type();
        // Extension headers (pax, GNU long names) are applied by the tar crate and are not entries.
        if kind.is_pax_global_extensions()
            || kind.is_pax_local_extensions()
            || kind.is_gnu_longname()
            || kind.is_gnu_longlink()
        {
            continue;
        }
        ctx.begin_entry()?;

        let raw = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let mode = entry.header().mode().ok();
        let Some(rel) = ctx.resolve(&raw)? else {
            continue;
        };

        if kind.is_dir() {
            ctx.make_directory(&rel, mode, &raw)?;
        } else if kind.is_file() || kind.is_contiguous() {
            ctx.write_file(&rel, &mut entry, mode, &raw)?;
        } else if kind.is_symlink() {
            let target = entry
                .link_name_bytes()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .ok_or_else(|| corrupt(format!("symbolic link {raw:?} has no target")))?;
            ctx.make_symlink(&rel, &target, &raw)?;
        } else if kind.is_hard_link() {
            let target = entry
                .link_name_bytes()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .ok_or_else(|| corrupt(format!("hard link {raw:?} has no target")))?;
            ctx.make_hard_link(&rel, &target, &raw)?;
        } else {
            return Err(unsafe_entry(
                &raw,
                "special files (devices, pipes, sparse files) are not allowed",
            ));
        }
        ctx.report();
    }
    if ctx.entries_seen == 0 {
        return Err(corrupt("it contains no files"));
    }
    Ok(ctx.summary)
}

/// Extract `archive_path` into the empty (or not yet existing) directory `dest`. This is the blocking
/// implementation, see [`extract`] for the async wrapper.
pub fn extract_sync(
    archive_path: &Path,
    dest: &Path,
    format: ArchiveFormat,
    options: &ExtractOptions,
    cancel: &CancellationToken,
    on_progress: &mut dyn FnMut(ExtractProgress),
) -> Result<ExtractSummary> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    prepare_destination(dest)?;

    match format {
        ArchiveFormat::Zip => extract_zip(archive_path, dest, options, cancel, on_progress),
        ArchiveFormat::TarGz => {
            let file = File::open(archive_path).at_path(archive_path)?;
            let decoder =
                flate2::read::MultiGzDecoder::new(BufReader::with_capacity(FILE_BUFFER, file));
            extract_tar(decoder, dest, options, cancel, on_progress)
        }
        ArchiveFormat::TarXz => {
            let file = File::open(archive_path).at_path(archive_path)?;
            let decoder = lzma_rust2::XzReader::new_mem_limit(
                BufReader::with_capacity(FILE_BUFFER, file),
                true,
                options.xz_memory_limit_kib,
            );
            extract_tar(decoder, dest, options, cancel, on_progress)
        }
    }
}

/// Async wrapper: runs [`extract_sync`] on a blocking thread so the async runtime is never stalled.
pub async fn extract(
    archive_path: PathBuf,
    dest: PathBuf,
    format: ArchiveFormat,
    options: ExtractOptions,
    cancel: CancellationToken,
    mut on_progress: impl FnMut(ExtractProgress) + Send + 'static,
) -> Result<ExtractSummary> {
    tokio::task::spawn_blocking(move || {
        extract_sync(
            &archive_path,
            &dest,
            format,
            &options,
            &cancel,
            &mut on_progress,
        )
    })
    .await
    .map_err(|err| LunaError::internal(format!("extraction task failed: {err}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(
        raw: &str,
        strip: usize,
        windows: bool,
    ) -> std::result::Result<Option<String>, String> {
        sanitize_entry_path(raw, strip, windows)
            .map(|p| p.map(|p| p.to_string_lossy().replace('\\', "/")))
    }

    #[test]
    fn accepts_ordinary_paths_and_normalises_separators() {
        for windows in [false, true] {
            assert_eq!(clean("bin/php", 0, windows), Ok(Some("bin/php".into())));
            assert_eq!(clean("./bin//php", 0, windows), Ok(Some("bin/php".into())));
            assert_eq!(
                clean("bin\\php.exe", 0, windows),
                Ok(Some("bin/php.exe".into()))
            );
            assert_eq!(clean("dir/", 0, windows), Ok(Some("dir".into())));
            assert_eq!(
                clean("ünï/файл.txt", 0, windows),
                Ok(Some("ünï/файл.txt".into()))
            );
        }
    }

    #[test]
    fn rejects_every_way_out_of_the_destination() {
        let hostile = [
            "../evil",
            "a/../../evil",
            "a/b/../../../evil",
            "..",
            "..\\evil",
            "a\\..\\..\\evil",
            "/etc/passwd",
            "\\windows\\system32",
            "\\\\server\\share\\x",
            "//server/share/x",
            "C:\\evil",
            "c:/evil",
            "C:evil",
            "a\0b",
        ];
        for raw in hostile {
            for windows in [false, true] {
                assert!(
                    sanitize_entry_path(raw, 0, windows).is_err(),
                    "{raw:?} (windows={windows}) must be rejected"
                );
            }
        }
    }

    #[test]
    fn validation_happens_before_stripping() {
        // Stripping two components must not make "../../x" look harmless.
        assert!(sanitize_entry_path("../../x", 2, false).is_err());
        assert!(sanitize_entry_path("a/../x", 1, false).is_err());
    }

    #[test]
    fn strips_leading_components_and_skips_the_wrapper_itself() {
        assert_eq!(
            clean("node-v24/bin/node", 1, false),
            Ok(Some("bin/node".into()))
        );
        assert_eq!(clean("node-v24/", 1, false), Ok(None));
        assert_eq!(clean("node-v24", 1, false), Ok(None));
        assert_eq!(clean("loose-file", 1, false), Ok(None));
        assert_eq!(clean("a/b/c", 2, false), Ok(Some("c".into())));
        assert_eq!(clean("a/b", 5, false), Ok(None));
    }

    #[test]
    fn windows_only_rules_apply_only_on_windows() {
        for raw in [
            "a/b:stream",
            "con",
            "NUL.txt",
            "dir/COM1",
            "lpt9.log",
            "bad|name",
            "trail.",
            "trail ",
            "q?",
        ] {
            assert!(
                sanitize_entry_path(raw, 0, true).is_err(),
                "{raw:?} must be rejected on Windows"
            );
            assert!(
                sanitize_entry_path(raw, 0, false).is_ok(),
                "{raw:?} is fine elsewhere"
            );
        }
        // Look-alikes that are legal everywhere.
        for raw in ["console", "com10", "lpt", "auxiliary.txt", "a.b"] {
            assert!(
                sanitize_entry_path(raw, 0, true).is_ok(),
                "{raw:?} must be accepted"
            );
        }
    }

    #[test]
    fn symlink_targets_must_stay_inside_the_destination() {
        let ok = |link: &str, target: &str| symlink_target_stays_inside(Path::new(link), target);
        assert!(ok("bin/npm", "../lib/node_modules/npm/bin/npm-cli.js").is_ok());
        assert!(ok("a/b/c", "../../x").is_ok());
        assert!(ok("link", "sub/dir").is_ok());
        assert!(ok("link", ".").is_ok());
        assert!(ok("lib/libfoo.so", "libfoo.so.1").is_ok());

        for (link, target) in [
            ("link", ".."),
            ("link", "../x"),
            ("a/link", "../../x"),
            ("a/b/link", "../../../x"),
            ("link", "/etc"),
            ("link", "\\windows"),
            ("link", "C:\\x"),
            ("link", ""),
            ("link", "a/../../x"),
            ("link", "x\0y"),
        ] {
            assert!(
                ok(link, target).is_err(),
                "{link} -> {target:?} must be rejected"
            );
        }
    }

    #[test]
    fn formats_are_recognised_by_name_and_registry_value() {
        assert_eq!(
            ArchiveFormat::from_file_name("php-8.3.14-nts-Win32-vs16-x64.zip"),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::from_file_name("NODE.TAR.GZ"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_file_name("x.tgz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_file_name("node-v24-linux-x64.tar.xz"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(
            ArchiveFormat::from_file_name("x.txz"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(ArchiveFormat::from_file_name("composer.phar"), None);
        assert_eq!(ArchiveFormat::from_file_name("x.tar"), None);
        assert_eq!(
            ArchiveFormat::from_registry("tar.xz"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(ArchiveFormat::from_registry("file"), None);
    }
}
