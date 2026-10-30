//! Opt-in test of the real HTTPS path (TLS handshake and the operating system's trust store), which
//! the loopback tests cannot cover. It needs internet access, so it is ignored by default:
//!
//!     cargo test -p luna-core --test download_online -- --ignored
//!
//! It downloads the Apache License 2.0 text, a stable file whose SHA-256 is pinned below.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use luna_core::download::{DownloadOptions, DownloadRequest, Downloader};
use luna_core::{ErrorCode, LunaHome};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const URL: &str = "https://www.apache.org/licenses/LICENSE-2.0.txt";
const SHA256: &str = "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30";
const SIZE: u64 = 11_358;

fn home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

#[tokio::test]
#[ignore = "needs internet access"]
async fn downloads_and_verifies_over_real_https() {
    let (_tmp, home) = home();
    let request = DownloadRequest {
        url: URL,
        file_name: "apache-license.txt",
        sha256: SHA256,
    };
    let path = Downloader::new(DownloadOptions::default())
        .unwrap()
        .download(&home, &request, &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(fs::metadata(path).unwrap().len(), SIZE);
}

#[tokio::test]
#[ignore = "needs internet access"]
async fn a_wrong_checksum_over_real_https_is_rejected_and_deleted() {
    let (_tmp, home) = home();
    let wrong = "0".repeat(64);
    let request = DownloadRequest {
        url: URL,
        file_name: "apache-license.txt",
        sha256: &wrong,
    };
    let error = Downloader::new(DownloadOptions::default())
        .unwrap()
        .download(&home, &request, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ChecksumMismatch);
    assert!(!home.downloads_dir().join("apache-license.txt").exists());
    assert!(
        !home
            .downloads_dir()
            .join("apache-license.txt.part")
            .exists()
    );
}
