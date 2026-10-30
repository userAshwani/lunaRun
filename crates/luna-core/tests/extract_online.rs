//! Opt-in tests against real, large archives: Node.js v24.22.2 (LTS), downloaded over HTTPS, verified
//! against Node's published SHA-256 sums, and extracted. They need internet access and about 70 MB of
//! downloads, so they are ignored by default:
//!
//!     cargo test -p luna-core --test extract_online -- --ignored --nocapture
//!
//! The expected counts come from Python's own `zipfile` and `tarfile` (liblzma) on the same files, so
//! they are an independent check of our zip, tar and XZ handling. The zip also has paths nested deep
//! enough (`node_modules/npm/...`) to show whether long Windows paths work.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::time::Instant;

use luna_core::LunaHome;
use luna_core::download::{DownloadOptions, DownloadRequest, Downloader};
use luna_core::extract::{ArchiveFormat, ExtractOptions, extract_sync};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const VERSION: &str = "v24.22.2";
const WIN_ZIP_SHA256: &str = "ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2";
const LINUX_XZ_SHA256: &str = "48cc0e21e048790e54c0cf5749c2725005313d652651c40b7a21feb9697aaf75";

// Reference numbers from Python (see the module docs).
const ZIP_FILES: u64 = 1_994;
const ZIP_BYTES: u64 = 107_353_198;
const XZ_REGULAR_FILES: u64 = 4_797;
const XZ_DIRECTORIES: u64 = 1_088;
const XZ_SYMLINKS: u64 = 3;
const XZ_BYTES: u64 = 201_760_169;

fn home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

async fn download(home: &LunaHome, file_name: &str, sha256: &str) -> std::path::PathBuf {
    let url = format!("https://nodejs.org/dist/{VERSION}/{file_name}");
    let request = DownloadRequest {
        url: &url,
        file_name,
        sha256,
    };
    Downloader::new(DownloadOptions::default())
        .unwrap()
        .download(home, &request, &CancellationToken::new(), |_| {})
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs internet access and downloads 38 MB"]
async fn real_node_windows_zip_extracts_completely() {
    let (_tmp, home) = home();
    let name = format!("node-{VERSION}-win-x64.zip");
    let archive = download(&home, &name, WIN_ZIP_SHA256).await;
    let dest = home.runtimes_dir().join("node").join(".staging-test");

    let started = Instant::now();
    let options = ExtractOptions {
        strip_components: 1,
        ..ExtractOptions::default()
    };
    let summary = extract_sync(
        &archive,
        &dest,
        ArchiveFormat::Zip,
        &options,
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap();
    eprintln!("zip: {summary:?} in {:?}", started.elapsed());

    assert_eq!(summary.files, ZIP_FILES);
    assert_eq!(summary.bytes, ZIP_BYTES);
    assert!(fs::metadata(dest.join("node.exe")).unwrap().len() > 50_000_000);
    assert!(
        dest.join("node_modules/npm/package.json").is_file(),
        "the deep npm tree was extracted"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "needs internet access and downloads 32 MB"]
async fn real_node_linux_tar_xz_extracts_completely() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, home) = home();
    let name = format!("node-{VERSION}-linux-x64.tar.xz");
    let archive = download(&home, &name, LINUX_XZ_SHA256).await;
    let dest = home.runtimes_dir().join("node").join(".staging-test");

    let started = Instant::now();
    let options = ExtractOptions {
        strip_components: 1,
        ..ExtractOptions::default()
    };
    let summary = extract_sync(
        &archive,
        &dest,
        ArchiveFormat::TarXz,
        &options,
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap();
    eprintln!("tar.xz: {summary:?} in {:?}", started.elapsed());

    assert_eq!(summary.files, XZ_REGULAR_FILES);
    assert_eq!(summary.symlinks, XZ_SYMLINKS);
    assert_eq!(summary.bytes, XZ_BYTES);
    let node = fs::metadata(dest.join("bin/node")).unwrap();
    assert!(node.len() > 50_000_000);
    assert_eq!(
        node.permissions().mode() & 0o111,
        0o111,
        "bin/node must stay executable"
    );
}

/// Runs on every OS, so Windows also proves our pure-Rust XZ decoder against a real 32 MB file (Windows
/// refuses the symlinks in this archive, so it cannot be fully extracted there).
#[tokio::test]
#[ignore = "needs internet access and downloads 32 MB"]
async fn real_node_tar_xz_decodes_to_exactly_what_python_sees() {
    let (_tmp, home) = home();
    let name = format!("node-{VERSION}-linux-x64.tar.xz");
    let archive = download(&home, &name, LINUX_XZ_SHA256).await;

    let started = Instant::now();
    let reader = lzma_rust2::XzReader::new(BufReader::new(File::open(archive).unwrap()), true);
    let mut tar = tar::Archive::new(reader);
    let (mut regular, mut directories, mut symlinks, mut bytes) = (0u64, 0u64, 0u64, 0u64);
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let kind = entry.header().entry_type();
        if kind.is_file() {
            regular += 1;
            let mut sink = Vec::new();
            bytes += entry.read_to_end(&mut sink).unwrap() as u64; // forces full decoding and CRC checks
        } else if kind.is_dir() {
            directories += 1;
        } else if kind.is_symlink() {
            symlinks += 1;
        }
    }
    eprintln!(
        "xz decode: {regular} files, {bytes} bytes in {:?}",
        started.elapsed()
    );

    assert_eq!(
        (regular, directories, symlinks, bytes),
        (XZ_REGULAR_FILES, XZ_DIRECTORIES, XZ_SYMLINKS, XZ_BYTES)
    );
}
