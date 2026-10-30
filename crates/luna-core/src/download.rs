//! Verified downloads (FR-RT-02, FR-RT-03, NFR-05).
//!
//! A file is fetched into `LUNA_HOME/downloads/<name>.part`, verified against its published SHA-256
//! and only then renamed to `<name>`, so a file with the final name is always complete and verified.
//!
//! - **Resumable:** an existing `.part` is continued with an HTTP `Range` request. If the server
//!   ignores it, the download restarts from zero. If the remote file changed in between, the final
//!   checksum catches it and the `.part` is deleted, so the next attempt starts clean.
//! - **Cancellable:** cancelling keeps the `.part` so the user can resume later.
//! - **Retried:** network errors and 5xx responses are retried with backoff (resuming from the bytes
//!   already on disk). 4xx responses are not retried.
//! - **HTTPS only,** including after redirects.
//! - **Bounded:** a download larger than `max_bytes` is aborted and its `.part` removed.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::{Client, StatusCode, Url, redirect};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::{LunaHome, component};

/// Behaviour switches. The defaults are what the app uses.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Accept plain `http://` for loopback addresses (127.0.0.1, ::1, localhost). Only tests set this.
    pub allow_loopback_http: bool,
    /// Ignore system proxy settings. Only tests set this, so a developer's proxy cannot break them.
    pub no_proxy: bool,
    /// Retries after the first attempt (so up to `1 + max_retries` attempts).
    pub max_retries: u32,
    /// Delay before the first retry; it doubles for each further retry.
    pub retry_base_delay: Duration,
    pub connect_timeout: Duration,
    /// Maximum silence between two chunks of the body.
    pub read_timeout: Duration,
    /// Abort when more than this many bytes arrive.
    pub max_bytes: u64,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            allow_loopback_http: false,
            no_proxy: false,
            max_retries: 3,
            retry_base_delay: Duration::from_secs(1),
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            max_bytes: 4 * 1024 * 1024 * 1024,
        }
    }
}

impl DownloadOptions {
    /// Options for tests against a local server: plain HTTP on loopback, no proxy, fast retries.
    #[doc(hidden)]
    pub fn loopback_test() -> Self {
        Self {
            allow_loopback_http: true,
            no_proxy: true,
            retry_base_delay: Duration::from_millis(10),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Bytes on disk so far, including bytes kept from an earlier attempt.
    pub bytes_done: u64,
    /// Total size when the server told us.
    pub bytes_total: Option<u64>,
}

pub struct DownloadRequest<'a> {
    pub url: &'a str,
    /// Name of the file inside `LUNA_HOME/downloads`: a single path component.
    pub file_name: &'a str,
    /// Expected SHA-256 as 64 hexadecimal characters (any case).
    pub sha256: &'a str,
}

pub struct Downloader {
    client: Client,
    options: DownloadOptions,
}

/// Outcome of one attempt, deciding whether the loop tries again.
enum Failure {
    Retry(LunaError),
    Fatal(LunaError),
}

fn is_loopback(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    )
}

fn url_allowed(url: &Url, allow_loopback_http: bool) -> bool {
    url.scheme() == "https" || (allow_loopback_http && url.scheme() == "http" && is_loopback(url))
}

fn network_error(message: impl Into<String>, url: &Url) -> LunaError {
    LunaError::new(ErrorCode::Network, message).with_details(json!({ "url": url.as_str() }))
}

fn cancelled() -> LunaError {
    LunaError::new(ErrorCode::Cancelled, "the download was cancelled")
}

impl Downloader {
    pub fn new(options: DownloadOptions) -> Result<Self> {
        let allow_loopback_http = options.allow_loopback_http;
        // Re-check every redirect, so an HTTPS URL cannot bounce to plain HTTP.
        let policy = redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if url_allowed(attempt.url(), allow_loopback_http) {
                attempt.follow()
            } else {
                attempt.error("redirect to a non-HTTPS address")
            }
        });

        let mut builder = Client::builder()
            .user_agent(concat!("lunaRun/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(options.connect_timeout)
            .read_timeout(options.read_timeout)
            .redirect(policy);
        if options.no_proxy {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|err| LunaError::internal(format!("cannot create the HTTP client: {err}")))?;
        Ok(Self { client, options })
    }

    /// Download, verify and return the path of the finished file inside `home`'s downloads folder.
    ///
    /// If a verified file with that name already exists it is reused without any network access.
    pub async fn download(
        &self,
        home: &LunaHome,
        request: &DownloadRequest<'_>,
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(Progress) + Send,
    ) -> Result<PathBuf> {
        let url = self.parse_url(request.url)?;
        let expected = normalize_sha256(request.sha256)?;
        let file_name = component(request.file_name)?;

        let dir = home.downloads_dir();
        fs::create_dir_all(&dir).at_path(&dir)?;
        let final_path = dir.join(file_name);
        let part_path = dir.join(format!("{file_name}.part"));

        if cancel.is_cancelled() {
            return Err(cancelled());
        }

        if final_path.exists() {
            if sha256_of(&final_path).await? == expected {
                return Ok(final_path);
            }
            home.safe_remove(&final_path)?; // stale or damaged: fetch it again
        }

        let mut retries_used = 0;
        loop {
            match self
                .attempt(home, &url, &part_path, cancel, &mut on_progress)
                .await
            {
                Ok(()) => break,
                Err(Failure::Fatal(error)) => return Err(error),
                Err(Failure::Retry(error)) => {
                    if retries_used >= self.options.max_retries {
                        return Err(error);
                    }
                    let delay = self.options.retry_base_delay * 2u32.saturating_pow(retries_used);
                    retries_used += 1;
                    tokio::select! {
                        () = cancel.cancelled() => return Err(cancelled()),
                        () = tokio::time::sleep(delay) => {}
                    }
                }
            }
        }

        let actual = sha256_of(&part_path).await?;
        if actual != expected {
            home.safe_remove(&part_path)?;
            return Err(LunaError::new(
                ErrorCode::ChecksumMismatch,
                format!("{file_name} does not match its published SHA-256, it was deleted"),
            )
            .with_details(json!({ "file": file_name, "expected": expected, "actual": actual })));
        }

        fs::rename(&part_path, &final_path).at_path(&final_path)?;
        Ok(final_path)
    }

    fn parse_url(&self, text: &str) -> Result<Url> {
        let invalid = |why: &str| {
            LunaError::new(
                ErrorCode::InvalidUrl,
                format!("{text:?} cannot be downloaded: {why}"),
            )
            .with_details(json!({ "url": text }))
        };
        let url = Url::parse(text).map_err(|_| invalid("it is not a valid URL"))?;
        if url_allowed(&url, self.options.allow_loopback_http) {
            Ok(url)
        } else {
            Err(invalid("only https:// addresses are allowed"))
        }
    }

    /// One try: continue the `.part` file if there is one, stream the body to disk.
    async fn attempt(
        &self,
        home: &LunaHome,
        url: &Url,
        part_path: &Path,
        cancel: &CancellationToken,
        on_progress: &mut (impl FnMut(Progress) + Send),
    ) -> std::result::Result<(), Failure> {
        let offset = fs::metadata(part_path).map_or(0, |meta| meta.len());

        let mut request = self.client.get(url.clone());
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let response = tokio::select! {
            () = cancel.cancelled() => return Err(Failure::Fatal(cancelled())),
            sent = request.send() => sent.map_err(|err| {
                let error = network_error(format!("request to {url} failed: {err}"), url);
                // A redirect refused by our own policy will not get better by trying again.
                if err.is_redirect() || err.is_builder() {
                    Failure::Fatal(error)
                } else {
                    Failure::Retry(error)
                }
            })?,
        };

        let status = response.status();
        // Where the body starts, and how big the whole file is.
        let (start, total) = match status {
            StatusCode::PARTIAL_CONTENT => {
                let (first, total) = parse_content_range(
                    response
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok()),
                )
                .ok_or_else(|| {
                    Failure::Fatal(network_error("the server sent a broken Content-Range", url))
                })?;
                if first != offset {
                    // The server resumed somewhere else than we asked. Start over.
                    home.safe_remove(part_path).map_err(Failure::Fatal)?;
                    return Err(Failure::Retry(network_error(
                        "the server resumed at an unexpected position",
                        url,
                    )));
                }
                (offset, total)
            }
            StatusCode::OK => (0, response.content_length()), // fresh, or the server ignored Range
            StatusCode::RANGE_NOT_SATISFIABLE => {
                // Our partial file does not fit the remote file anymore. Start over.
                home.safe_remove(part_path).map_err(Failure::Fatal)?;
                return Err(Failure::Retry(network_error(
                    "the partial download no longer matches the server, restarting",
                    url,
                )));
            }
            other => {
                let error = network_error(format!("{url} answered HTTP {other}"), url)
                    .with_details(json!({ "url": url.as_str(), "status": other.as_u16() }));
                return Err(if other.is_server_error() {
                    Failure::Retry(error)
                } else {
                    Failure::Fatal(error)
                });
            }
        };

        let io = |err: std::io::Error| Failure::Fatal(LunaError::from(err));
        let mut file = if start > 0 {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(part_path)
                .await
                .map_err(io)?
        } else {
            tokio::fs::File::create(part_path).await.map_err(io)? // also truncates stale bytes
        };

        let mut done = start;
        on_progress(Progress {
            bytes_done: done,
            bytes_total: total,
        });

        let mut body = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                () = cancel.cancelled() => {
                    let _ = file.flush().await; // keep what we have for a later resume
                    return Err(Failure::Fatal(cancelled()));
                }
                next = body.next() => next,
            };
            match chunk {
                Some(Ok(bytes)) => {
                    done += bytes.len() as u64;
                    if done > self.options.max_bytes {
                        drop(file);
                        home.safe_remove(part_path).map_err(Failure::Fatal)?;
                        return Err(Failure::Fatal(network_error(
                            format!(
                                "the download is larger than the allowed {} bytes",
                                self.options.max_bytes
                            ),
                            url,
                        )));
                    }
                    file.write_all(&bytes).await.map_err(io)?;
                    on_progress(Progress {
                        bytes_done: done,
                        bytes_total: total,
                    });
                }
                Some(Err(err)) => {
                    let _ = file.flush().await;
                    return Err(Failure::Retry(network_error(
                        format!("the connection to {url} was interrupted: {err}"),
                        url,
                    )));
                }
                None => break,
            }
        }
        file.flush().await.map_err(io)?;

        match total {
            Some(total) if done != total => Err(Failure::Retry(network_error(
                format!("the connection closed after {done} of {total} bytes"),
                url,
            ))),
            _ => Ok(()),
        }
    }
}

/// `bytes 100-199/1000` gives `(100, Some(1000))`. A `*` total gives `None`.
fn parse_content_range(header: Option<&str>) -> Option<(u64, Option<u64>)> {
    let rest = header?.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (first, _last) = range.split_once('-')?;
    let total = if total.trim() == "*" {
        None
    } else {
        Some(total.trim().parse().ok()?)
    };
    Some((first.trim().parse().ok()?, total))
}

fn normalize_sha256(text: &str) -> Result<String> {
    let text = text.trim();
    if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(text.to_ascii_lowercase())
    } else {
        Err(LunaError::internal(format!(
            "{text:?} is not a SHA-256 (expected 64 hexadecimal characters)"
        )))
    }
}

/// Lowercase hexadecimal, the form SHA-256 sums are published in.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}"); // writing to a String cannot fail
            out
        })
}

/// Hash a file on a blocking thread, so large files do not stall the async runtime.
async fn sha256_of(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let shown = path.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(to_hex(&hasher.finalize()))
    })
    .await
    .map_err(|err| LunaError::internal(format!("hashing task failed: {err}")))?
    .at_path(&shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_is_parsed() {
        assert_eq!(
            parse_content_range(Some("bytes 100-199/1000")),
            Some((100, Some(1000)))
        );
        assert_eq!(parse_content_range(Some("bytes 0-0/*")), Some((0, None)));
        for bad in ["", "bytes", "items 1-2/3", "bytes a-b/c", "bytes 1/3"] {
            assert_eq!(parse_content_range(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(parse_content_range(None), None);
    }

    #[test]
    fn hex_encoding_matches_known_sha256_vectors() {
        let hash = |data: &[u8]| to_hex(&Sha256::digest(data));
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_must_be_64_hex_characters_and_is_lowercased() {
        let upper = "A".repeat(64);
        assert_eq!(normalize_sha256(&upper).unwrap(), "a".repeat(64));
        assert_eq!(
            normalize_sha256(&format!("  {}\n", "b".repeat(64))).unwrap(),
            "b".repeat(64)
        );
        for bad in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            assert!(normalize_sha256(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_https_and_optionally_loopback_http_are_allowed() {
        let allowed =
            |text: &str, loopback: bool| url_allowed(&Url::parse(text).unwrap(), loopback);
        assert!(allowed("https://example.com/a.zip", false));
        assert!(!allowed("http://example.com/a.zip", false));
        assert!(
            !allowed("http://example.com/a.zip", true),
            "loopback flag is not a free pass"
        );
        assert!(!allowed("http://127.0.0.1:8080/a", false));
        assert!(allowed("http://127.0.0.1:8080/a", true));
        assert!(allowed("http://localhost/a", true));
        assert!(!allowed("ftp://example.com/a", true));
        assert!(!allowed("file:///etc/passwd", true));
    }
}
