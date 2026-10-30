//! Download engine tests against a local HTTP server. Every test uses a data folder whose path has a
//! space and non-ASCII characters (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use luna_core::download::{DownloadOptions, DownloadRequest, Downloader, Progress};
use luna_core::{ErrorCode, LunaHome};
use sha2::{Digest, Sha256};
use support::{Server, respond, send_part_of, serve, serve_ignoring_range};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const FILE_NAME: &str = "php-8.3.14-ü.zip";

fn home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn downloader() -> Downloader {
    Downloader::new(DownloadOptions::loopback_test()).unwrap()
}

fn part_path(home: &LunaHome) -> PathBuf {
    home.downloads_dir().join(format!("{FILE_NAME}.part"))
}

fn final_path(home: &LunaHome) -> PathBuf {
    home.downloads_dir().join(FILE_NAME)
}

fn names_in_downloads(home: &LunaHome) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(home.downloads_dir())
        .map(|dir| {
            dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

async fn fetch(
    home: &LunaHome,
    url: &str,
    sha256: &str,
    cancel: &CancellationToken,
) -> luna_core::Result<PathBuf> {
    let request = DownloadRequest {
        url,
        file_name: FILE_NAME,
        sha256,
    };
    downloader().download(home, &request, cancel, |_| {}).await
}

#[tokio::test]
async fn downloads_verifies_and_reports_progress() {
    let (_tmp, home) = home();
    let data = support::sample_body(300_000);
    let served = data.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));

    let mut events: Vec<Progress> = Vec::new();
    let url = server.url("/file");
    let request = DownloadRequest {
        url: &url,
        file_name: FILE_NAME,
        sha256: &sha256_hex(&data),
    };
    let path = downloader()
        .download(&home, &request, &CancellationToken::new(), |p| {
            events.push(p)
        })
        .await
        .unwrap();

    assert_eq!(path, final_path(&home));
    assert_eq!(fs::read(&path).unwrap(), data);
    assert_eq!(
        names_in_downloads(&home),
        vec![FILE_NAME.to_owned()],
        "no .part left behind"
    );

    let total = Some(data.len() as u64);
    assert_eq!(events.first().unwrap().bytes_done, 0);
    assert!(events.iter().all(|e| e.bytes_total == total));
    assert!(
        events
            .windows(2)
            .all(|w| w[0].bytes_done <= w[1].bytes_done),
        "progress never goes back"
    );
    assert_eq!(events.last().unwrap().bytes_done, data.len() as u64);
}

#[tokio::test]
async fn accepts_an_uppercase_checksum() {
    let (_tmp, home) = home();
    let data = support::sample_body(1_000);
    let served = data.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));
    let sha = sha256_hex(&data).to_uppercase();
    fetch(&home, &server.url("/file"), &sha, &CancellationToken::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn checksum_mismatch_deletes_the_download() {
    let (_tmp, home) = home();
    let data = support::sample_body(50_000);
    let server = Server::start(move |req, _, stream| serve(stream, req, &data));

    let wrong = sha256_hex(b"something else");
    let error = fetch(
        &home,
        &server.url("/file"),
        &wrong,
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::ChecksumMismatch);
    let details = error.details.unwrap();
    assert_eq!(details["expected"], wrong);
    assert_ne!(details["actual"], wrong);
    assert!(
        names_in_downloads(&home).is_empty(),
        "neither .part nor final file remains"
    );
}

#[tokio::test]
async fn reuses_a_verified_file_without_touching_the_network() {
    let (_tmp, home) = home();
    let data = support::sample_body(10_000);
    fs::create_dir_all(home.downloads_dir()).unwrap();
    fs::write(final_path(&home), &data).unwrap();

    // Nothing listens here: any network attempt would fail the test.
    let path = fetch(
        &home,
        "http://127.0.0.1:9/never",
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(path).unwrap(), data);
}

#[tokio::test]
async fn replaces_a_damaged_existing_file() {
    let (_tmp, home) = home();
    let data = support::sample_body(10_000);
    fs::create_dir_all(home.downloads_dir()).unwrap();
    fs::write(final_path(&home), b"damaged").unwrap();

    let served = data.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));
    let path = fetch(
        &home,
        &server.url("/file"),
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(fs::read(path).unwrap(), data);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn resumes_from_the_bytes_already_on_disk() {
    let (_tmp, home) = home();
    let data = support::sample_body(200_000);
    let already = 80_000;
    fs::create_dir_all(home.downloads_dir()).unwrap();
    fs::write(part_path(&home), &data[..already]).unwrap();

    let served = data.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));
    let mut events: Vec<Progress> = Vec::new();
    let url = server.url("/file");
    let request = DownloadRequest {
        url: &url,
        file_name: FILE_NAME,
        sha256: &sha256_hex(&data),
    };
    let path = downloader()
        .download(&home, &request, &CancellationToken::new(), |p| {
            events.push(p)
        })
        .await
        .unwrap();

    assert_eq!(fs::read(path).unwrap(), data);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].range.as_deref(),
        Some(format!("bytes={already}-").as_str())
    );
    assert_eq!(
        events.first().unwrap().bytes_done,
        already as u64,
        "progress starts where we left off"
    );
    assert_eq!(events.first().unwrap().bytes_total, Some(data.len() as u64));
}

#[tokio::test]
async fn restarts_cleanly_when_the_server_ignores_range() {
    let (_tmp, home) = home();
    let data = support::sample_body(120_000);
    fs::create_dir_all(home.downloads_dir()).unwrap();
    fs::write(part_path(&home), &data[..40_000]).unwrap();

    let served = data.clone();
    let server = Server::start(move |_, _, stream| serve_ignoring_range(stream, &served));
    let path = fetch(
        &home,
        &server.url("/file"),
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    // If the 40 000 old bytes were kept and the full body appended, the checksum would not match.
    assert_eq!(fs::read(path).unwrap(), data);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn restarts_when_the_partial_file_is_longer_than_the_remote_file() {
    let (_tmp, home) = home();
    let data = support::sample_body(5_000);
    fs::create_dir_all(home.downloads_dir()).unwrap();
    fs::write(part_path(&home), support::sample_body(9_000)).unwrap(); // stale, from a bigger file

    let served = data.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));
    let path = fetch(
        &home,
        &server.url("/file"),
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(fs::read(path).unwrap(), data);
    assert_eq!(
        server.request_count(),
        2,
        "416 first, then a fresh download"
    );
}

#[tokio::test]
async fn retries_server_errors_and_then_succeeds() {
    let (_tmp, home) = home();
    let data = support::sample_body(20_000);
    let served = data.clone();
    let server = Server::start(move |req, nth, stream| {
        if nth < 2 {
            respond(stream, "503 Service Unavailable", &[], b"busy");
        } else {
            serve(stream, req, &served);
        }
    });
    let path = fetch(
        &home,
        &server.url("/file"),
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(path).unwrap(), data);
    assert_eq!(server.request_count(), 3);
}

#[tokio::test]
async fn gives_up_after_the_retry_limit() {
    let (_tmp, home) = home();
    let server =
        Server::start(|_, _, stream| respond(stream, "500 Internal Server Error", &[], b""));
    let options = DownloadOptions {
        max_retries: 2,
        ..DownloadOptions::loopback_test()
    };
    let url = server.url("/file");
    let request = DownloadRequest {
        url: &url,
        file_name: FILE_NAME,
        sha256: &sha256_hex(b"x"),
    };

    let error = Downloader::new(options)
        .unwrap()
        .download(&home, &request, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Network);
    assert_eq!(error.details.unwrap()["status"], 500);
    assert_eq!(server.request_count(), 3, "one try plus two retries");
}

#[tokio::test]
async fn does_not_retry_client_errors() {
    let (_tmp, home) = home();
    let server = Server::start(|_, _, stream| respond(stream, "404 Not Found", &[], b"nope"));
    let error = fetch(
        &home,
        &server.url("/missing"),
        &sha256_hex(b"x"),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::Network);
    assert_eq!(error.details.unwrap()["status"], 404);
    assert_eq!(server.request_count(), 1);
    assert!(names_in_downloads(&home).is_empty());
}

#[tokio::test]
async fn resumes_after_the_connection_drops_mid_download() {
    let (_tmp, home) = home();
    let data = support::sample_body(100_000);
    let half = 40_000;
    let served = data.clone();
    let server = Server::start(move |req, nth, stream| {
        if nth == 0 {
            // Promise everything, deliver 40 000 bytes, then hang up.
            send_part_of(stream, &served, 0, half, None);
        } else {
            serve(stream, req, &served);
        }
    });

    let path = fetch(
        &home,
        &server.url("/file"),
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(fs::read(path).unwrap(), data);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].range.as_deref(),
        Some(format!("bytes={half}-").as_str()),
        "the retry resumed"
    );
}

#[tokio::test]
async fn cancelling_keeps_the_partial_file_for_a_later_resume() {
    let (_tmp, home) = home();
    let data = support::sample_body(100_000);
    let half = 50_000;
    let served = data.clone();
    // Sends half of the body, then goes quiet for a long time.
    let server = Server::start(move |_, _, stream| {
        send_part_of(stream, &served, 0, half, Some(Duration::from_secs(5)));
    });

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let url = server.url("/file");
    let request = DownloadRequest {
        url: &url,
        file_name: FILE_NAME,
        sha256: &sha256_hex(&data),
    };
    let error = downloader()
        .download(&home, &request, &cancel, move |p| {
            if p.bytes_done > 0 {
                trigger.cancel();
            }
        })
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Cancelled);
    assert!(!final_path(&home).exists());
    let kept = fs::metadata(part_path(&home)).unwrap().len();
    assert!(
        kept > 0 && kept <= half as u64,
        "kept {kept} bytes of the {half} that were sent"
    );
    assert_eq!(
        server.request_count(),
        1,
        "cancelling must not trigger a retry"
    );
}

#[tokio::test]
async fn an_already_cancelled_token_stops_before_any_request() {
    let (_tmp, home) = home();
    let server = Server::start(|_, _, stream| respond(stream, "200 OK", &[], b"x"));
    let cancel = CancellationToken::new();
    cancel.cancel();

    let error = fetch(&home, &server.url("/file"), &sha256_hex(b"x"), &cancel)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(server.request_count(), 0);
}

#[tokio::test]
async fn follows_a_redirect_to_the_same_loopback_server() {
    let (_tmp, home) = home();
    let data = support::sample_body(10_000);
    let served = data.clone();
    let server = Server::start(move |req, _, stream| {
        if req.path == "/old" {
            respond(
                stream,
                "302 Found",
                &[("Location", "/file".to_owned())],
                b"",
            );
        } else {
            serve(stream, req, &served);
        }
    });
    let path = fetch(
        &home,
        &server.url("/old"),
        &sha256_hex(&data),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(path).unwrap(), data);
}

#[tokio::test]
async fn refuses_a_redirect_to_plain_http_on_another_host() {
    let (_tmp, home) = home();
    // `.invalid` never resolves, so even a broken policy cannot reach the real internet from a test.
    let server = Server::start(|_, _, stream| {
        respond(
            stream,
            "302 Found",
            &[("Location", "http://downloads.invalid/evil.bin".to_owned())],
            b"",
        );
    });
    let error = fetch(
        &home,
        &server.url("/file"),
        &sha256_hex(b"x"),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::Network);
    // The refusal itself, not a later connection failure to the redirect target.
    assert!(
        error.message.contains("redirect"),
        "unexpected error: {}",
        error.message
    );
    assert_eq!(
        server.request_count(),
        1,
        "a refused redirect is not retried"
    );
    assert!(names_in_downloads(&home).is_empty());
}

#[tokio::test]
async fn aborts_and_cleans_up_when_the_download_is_larger_than_allowed() {
    let (_tmp, home) = home();
    let data = support::sample_body(200_000);
    let server = Server::start(move |req, _, stream| serve(stream, req, &data));
    let options = DownloadOptions {
        max_bytes: 50_000,
        ..DownloadOptions::loopback_test()
    };
    let url = server.url("/file");
    let request = DownloadRequest {
        url: &url,
        file_name: FILE_NAME,
        sha256: &sha256_hex(b"x"),
    };

    let error = Downloader::new(options)
        .unwrap()
        .download(&home, &request, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Network);
    assert!(error.message.contains("larger than"));
    assert_eq!(server.request_count(), 1);
    assert!(names_in_downloads(&home).is_empty());
}

#[tokio::test]
async fn rejects_unsafe_input_before_any_network_access() {
    let (_tmp, home) = home();
    let strict = Downloader::new(DownloadOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let sha = sha256_hex(b"x");

    let code_for = async |url: &str, file_name: &str, sha256: &str| {
        let request = DownloadRequest {
            url,
            file_name,
            sha256,
        };
        strict
            .download(&home, &request, &cancel, |_| {})
            .await
            .unwrap_err()
            .code
    };

    assert_eq!(
        code_for("http://example.com/a.zip", "a.zip", &sha).await,
        ErrorCode::InvalidUrl
    );
    assert_eq!(
        code_for("http://127.0.0.1:9/a.zip", "a.zip", &sha).await,
        ErrorCode::InvalidUrl
    );
    assert_eq!(
        code_for("ftp://example.com/a.zip", "a.zip", &sha).await,
        ErrorCode::InvalidUrl
    );
    assert_eq!(
        code_for("not a url", "a.zip", &sha).await,
        ErrorCode::InvalidUrl
    );
    assert_eq!(
        code_for("https://example.com/a.zip", "../evil", &sha).await,
        ErrorCode::InvalidPath
    );
    assert_eq!(
        code_for("https://example.com/a.zip", "a/b.zip", &sha).await,
        ErrorCode::InvalidPath
    );
    assert_eq!(
        code_for("https://example.com/a.zip", "a.zip", "not-a-sha").await,
        ErrorCode::Internal
    );
}
