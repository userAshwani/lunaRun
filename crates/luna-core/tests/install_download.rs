//! The real fetcher the installer uses, `downloader_fetcher`, against a local HTTP server: the download
//! engine's own behaviour is tested in `tests/download.rs`, this only proves the installer's wrapper passes
//! the request through correctly. The data folder has a space and non-ASCII characters (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::{Arc, Mutex};

use luna_core::download::{DownloadOptions, Downloader, Progress};
use luna_core::runtimes::{FetchRequest, downloader_fetcher};
use luna_core::{ErrorCode, LunaHome};
use sha2::{Digest, Sha256};
use support::{Server, respond, serve};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn setup() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

fn fetcher(home: &LunaHome) -> luna_core::runtimes::Fetcher {
    downloader_fetcher(
        home.clone(),
        Downloader::new(DownloadOptions::loopback_test()).unwrap(),
    )
}

#[tokio::test]
async fn the_file_arrives_verified_under_the_requested_name_with_progress() {
    let (_tmp, home) = setup();
    let data = support::sample_body(300_000);
    let served = data.clone();
    let server = Server::start(move |request, _, stream| serve(stream, request, &served));
    let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let record = Arc::clone(&seen);

    let path = fetcher(&home)(FetchRequest {
        url: server.url("/php.zip"),
        file_name: "abcdef123456-php.zip".to_owned(),
        sha256: sha256_hex(&data),
        cancel: CancellationToken::new(),
        on_progress: Box::new(move |p| record.lock().unwrap().push(p)),
    })
    .await
    .unwrap();

    assert_eq!(path, home.downloads_dir().join("abcdef123456-php.zip"));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    let last = *seen.lock().unwrap().last().unwrap();
    assert_eq!(last.bytes_done, data.len() as u64);
    assert_eq!(last.bytes_total, Some(data.len() as u64));
}

#[tokio::test]
async fn a_wrong_checksum_is_refused_and_the_file_is_not_kept() {
    let (_tmp, home) = setup();
    let data = support::sample_body(10_000);
    let served = data.clone();
    let server = Server::start(move |request, _, stream| serve(stream, request, &served));

    let err = fetcher(&home)(FetchRequest {
        url: server.url("/a.zip"),
        file_name: "x-a.zip".to_owned(),
        sha256: "0".repeat(64),
        cancel: CancellationToken::new(),
        on_progress: Box::new(|_| {}),
    })
    .await
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(!home.downloads_dir().join("x-a.zip").exists());
}

#[tokio::test]
async fn an_http_error_and_a_cancel_come_back_as_their_own_codes() {
    let (_tmp, home) = setup();
    let server = Server::start(|_, _, stream| respond(stream, "404 Not Found", &[], b"no"));
    let missing = fetcher(&home)(FetchRequest {
        url: server.url("/gone.zip"),
        file_name: "x-gone.zip".to_owned(),
        sha256: "0".repeat(64),
        cancel: CancellationToken::new(),
        on_progress: Box::new(|_| {}),
    })
    .await
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::Network);

    let cancel = CancellationToken::new();
    cancel.cancel();
    let cancelled = fetcher(&home)(FetchRequest {
        url: server.url("/gone.zip"),
        file_name: "x-gone.zip".to_owned(),
        sha256: "0".repeat(64),
        cancel,
        on_progress: Box::new(|_| {}),
    })
    .await
    .unwrap_err();
    assert_eq!(cancelled.code, ErrorCode::Cancelled);
}
