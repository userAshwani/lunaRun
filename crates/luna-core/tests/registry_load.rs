//! Fetch, cache and bundled fallback of the registry, against a local HTTP server. Every test uses a
//! data folder whose path has a space and non-ASCII characters (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use luna_core::download::{DownloadOptions, Downloader};
use luna_core::registry::{
    LoadedRegistry, MAX_REGISTRY_BYTES, Registry, RegistrySource, bundled_registry, cache_path,
    load_registry,
};
use luna_core::{ErrorCode, LunaHome};
use support::{Server, respond};
use tempfile::TempDir;

fn home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

fn downloader() -> Downloader {
    Downloader::new(DownloadOptions::loopback_test()).unwrap()
}

/// A valid registry whose only runtime has one version, so a test can tell which copy it got.
fn registry_with(version: &str) -> String {
    format!(
        r#"{{ "schema": 1, "runtimes": {{ "php": {{ "displayName": "PHP",
            "versions": [{{ "version": "{version}", "artifacts": [] }}] }} }} }}"#
    )
}

fn newest_php(loaded: &LoadedRegistry) -> String {
    loaded.registry.runtime("php").unwrap().releases[0]
        .version
        .to_string()
}

/// A server whose answer the test can change between loads.
struct Switchable {
    server: Server,
    answer: Arc<Mutex<(String, Vec<u8>)>>,
}

impl Switchable {
    fn start(status: &str, body: &str) -> Self {
        let answer = Arc::new(Mutex::new((status.to_owned(), body.as_bytes().to_vec())));
        let shared = Arc::clone(&answer);
        let server = Server::start(move |_, _, stream| {
            let (status, body) = shared.lock().unwrap().clone();
            respond(stream, &status, &[], &body);
        });
        Self { server, answer }
    }

    fn set(&self, status: &str, body: &str) {
        *self.answer.lock().unwrap() = (status.to_owned(), body.as_bytes().to_vec());
    }

    fn url(&self) -> String {
        self.server.url("/registry.json")
    }
}

/// An address on which nothing listens, so connecting fails at once.
fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/registry.json", listener.local_addr().unwrap());
    drop(listener);
    url
}

async fn load(home: &LunaHome, location: &str) -> LoadedRegistry {
    load_registry(home, &downloader(), location).await.unwrap()
}

fn problem_code(loaded: &LoadedRegistry) -> ErrorCode {
    loaded.problem.as_ref().expect("a problem is reported").code
}

fn cached_text(home: &LunaHome) -> String {
    fs::read_to_string(cache_path(home)).unwrap()
}

#[test]
fn the_bundled_registry_is_valid() {
    // The last resort must always work. A broken bundled file would fail every offline start.
    let bundled = bundled_registry().unwrap();

    // It is the generated registry, so an offline first start still has something to install.
    let ids: Vec<&str> = bundled.runtime_ids().collect();
    assert_eq!(
        ids,
        ["caddy", "composer", "mariadb", "mysql", "node", "php"]
    );
    assert!(bundled.runtime("php").unwrap().releases.len() >= 6);

    // And it is exactly the file in the repository, never a second copy that could drift.
    let file = Registry::parse(include_str!("../../../registry/registry.json")).unwrap();
    assert_eq!(bundled, file);
}

#[tokio::test]
async fn a_fresh_fetch_is_used_and_saved() {
    let (_tmp, home) = home();
    let body = registry_with("8.3.14");
    let server = Switchable::start("200 OK", &body);

    let loaded = load(&home, &server.url()).await;

    assert_eq!(loaded.source, RegistrySource::Fresh);
    assert!(loaded.problem.is_none());
    assert_eq!(newest_php(&loaded), "8.3.14");
    assert_eq!(cached_text(&home), body);
}

#[tokio::test]
async fn offline_uses_the_saved_copy_and_says_why() {
    let (_tmp, home) = home();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));
    load(&home, &server.url()).await;

    let loaded = load(&home, &dead_url()).await;

    assert!(matches!(
        loaded.source,
        RegistrySource::Cache { age: Some(_) }
    ));
    assert_eq!(newest_php(&loaded), "8.3.14");
    assert_eq!(problem_code(&loaded), ErrorCode::Network);
}

#[tokio::test]
async fn a_server_error_or_missing_file_keeps_the_saved_copy() {
    let (_tmp, home) = home();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));
    load(&home, &server.url()).await;

    for status in ["503 Service Unavailable", "404 Not Found"] {
        server.set(status, "<html>error page</html>");
        let loaded = load(&home, &server.url()).await;
        assert!(
            matches!(loaded.source, RegistrySource::Cache { .. }),
            "{status}"
        );
        assert_eq!(problem_code(&loaded), ErrorCode::Network, "{status}");
    }
    assert_eq!(cached_text(&home), registry_with("8.3.14"));
}

#[tokio::test]
async fn a_bad_download_never_replaces_a_good_saved_copy() {
    let (_tmp, home) = home();
    let good = registry_with("8.3.14");
    let server = Switchable::start("200 OK", &good);
    load(&home, &server.url()).await;

    // Every kind of bad body: an error page served as 200, truncated JSON, wrong shape, and a
    // registry that breaks a rule (an http:// artifact).
    let http_artifact = r#"{ "schema": 1, "runtimes": { "php": { "displayName": "PHP", "versions": [
        { "version": "9.9.9", "artifacts": [ { "os": "linux", "arch": "x86_64", "format": "zip",
          "url": "http://example.com/a.zip", "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1 } ] } ] } } }"#;
    for body in [
        "<html>captive portal</html>",
        r#"{ "schema": 1, "runtimes": { "php": "#,
        r#"{ "nothing": true }"#,
        http_artifact,
        "",
    ] {
        server.set("200 OK", body);
        let loaded = load(&home, &server.url()).await;
        assert!(
            matches!(loaded.source, RegistrySource::Cache { .. }),
            "{body}"
        );
        assert_eq!(newest_php(&loaded), "8.3.14");
        assert_eq!(problem_code(&loaded), ErrorCode::RegistryInvalid, "{body}");
        assert_eq!(cached_text(&home), good, "cache untouched after {body:?}");
    }
}

#[tokio::test]
async fn a_registry_for_a_newer_lunarun_keeps_the_saved_copy_and_asks_for_an_update() {
    let (_tmp, home) = home();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));
    load(&home, &server.url()).await;

    server.set(
        "200 OK",
        r#"{ "schema": 2, "runtimes": [ "a new shape" ] }"#,
    );
    let loaded = load(&home, &server.url()).await;

    assert!(matches!(loaded.source, RegistrySource::Cache { .. }));
    assert_eq!(problem_code(&loaded), ErrorCode::RegistryUnsupportedSchema);
    assert_eq!(cached_text(&home), registry_with("8.3.14"));
}

#[tokio::test]
async fn an_oversized_answer_is_refused() {
    let (_tmp, home) = home();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));
    load(&home, &server.url()).await;

    let padding = " ".repeat(usize::try_from(MAX_REGISTRY_BYTES).unwrap() + 1);
    server.set("200 OK", &format!("{}{padding}", registry_with("9.0.0")));
    let loaded = load(&home, &server.url()).await;

    assert!(matches!(loaded.source, RegistrySource::Cache { .. }));
    assert_eq!(problem_code(&loaded), ErrorCode::Network);
    assert_eq!(newest_php(&loaded), "8.3.14");
}

#[tokio::test]
async fn with_nothing_saved_the_bundled_registry_is_used() {
    let (_tmp, home) = home();

    for location in [dead_url(), "http://example.com/registry.json".to_owned()] {
        // The second address is plain http on a public host: refused before any network access.
        let loaded = load(&home, &location).await;
        assert_eq!(loaded.source, RegistrySource::Bundled, "{location}");
        assert!(loaded.problem.is_some());
        assert_eq!(
            loaded.registry,
            bundled_registry().unwrap(),
            "{location} gives exactly the bundled registry"
        );
    }
    assert!(
        !cache_path(&home).exists(),
        "a fallback never writes the cache"
    );
}

#[tokio::test]
async fn a_damaged_saved_copy_is_skipped() {
    let (_tmp, home) = home();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));
    load(&home, &server.url()).await;
    fs::write(cache_path(&home), "{ half a file").unwrap();

    let loaded = load(&home, &dead_url()).await;
    assert_eq!(loaded.source, RegistrySource::Bundled);

    // The next successful fetch repairs it.
    let loaded = load(&home, &server.url()).await;
    assert_eq!(loaded.source, RegistrySource::Fresh);
    assert_eq!(cached_text(&home), registry_with("8.3.14"));
}

#[tokio::test]
async fn a_local_file_override_works_by_path_and_by_file_url_and_is_not_cached() {
    let (tmp, home) = home();
    let dir = tmp.path().join("dev regîstry");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("registry.json");
    fs::write(&file, registry_with("8.4.1")).unwrap();

    let by_path = load(&home, file.to_str().unwrap()).await;
    assert_eq!(by_path.source, RegistrySource::Fresh);
    assert_eq!(newest_php(&by_path), "8.4.1");

    let url = reqwest::Url::from_file_path(&file).unwrap();
    let by_url = load(&home, url.as_str()).await;
    assert_eq!(by_url.source, RegistrySource::Fresh);
    assert_eq!(newest_php(&by_url), "8.4.1");

    assert!(!cache_path(&home).exists());
}

#[tokio::test]
async fn a_missing_or_invalid_override_file_falls_back_and_says_why() {
    let (tmp, home) = home();

    let missing = tmp.path().join("nope.json");
    let loaded = load(&home, missing.to_str().unwrap()).await;
    assert_eq!(loaded.source, RegistrySource::Bundled);
    assert_eq!(problem_code(&loaded), ErrorCode::NotFound);

    let broken = tmp.path().join("broken.json");
    fs::write(&broken, "not json").unwrap();
    let loaded = load(&home, broken.to_str().unwrap()).await;
    assert_eq!(problem_code(&loaded), ErrorCode::RegistryInvalid);

    let loaded = load(&home, "file://not-a-valid-host/x").await;
    assert_eq!(loaded.source, RegistrySource::Bundled);
    assert!(loaded.problem.is_some());
}

#[tokio::test]
async fn a_cache_that_cannot_be_written_is_reported_but_the_fresh_registry_is_still_used() {
    let (_tmp, home) = home();
    // A folder where the cache file should go makes the final rename fail on every platform.
    fs::create_dir_all(cache_path(&home)).unwrap();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));

    let loaded = load(&home, &server.url()).await;

    assert_eq!(loaded.source, RegistrySource::Fresh);
    assert_eq!(newest_php(&loaded), "8.3.14");
    assert!(loaded.problem.is_some(), "the failed save is not hidden");
}

#[tokio::test]
async fn a_failed_save_leaves_no_temporary_file_behind() {
    let (_tmp, home) = home();
    fs::create_dir_all(cache_path(&home)).unwrap();
    let server = Switchable::start("200 OK", &registry_with("8.3.14"));
    load(&home, &server.url()).await;

    let names: Vec<String> = fs::read_dir(home.cache_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["registry.json"], "only the blocking folder remains");
}

#[tokio::test]
async fn an_oversized_body_without_an_announced_length_is_refused_too() {
    let (_tmp, home) = home();
    // No Content-Length: the size can only be known by counting what arrives.
    let server = Server::start(|_, _, stream| {
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK
Connection: close

",
        );
        let chunk = vec![b' '; 1024 * 1024];
        for _ in 0..=MAX_REGISTRY_BYTES / (1024 * 1024) {
            if stream.write_all(&chunk).is_err() {
                return;
            }
        }
    });

    let loaded = load(&home, &server.url("/registry.json")).await;

    assert_eq!(loaded.source, RegistrySource::Bundled);
    assert_eq!(problem_code(&loaded), ErrorCode::Network);
}
