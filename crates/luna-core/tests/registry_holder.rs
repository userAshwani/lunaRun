//! The registry the running app holds: starts built-in, refreshes in the background, explains where the list
//! came from. Every data folder has a space and non-ASCII characters in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::net::TcpListener;
use std::time::Duration;

use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::registry::{CalDate, RegistryHolder, RegistrySourceKind};
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

/// A valid registry with one runtime, so a test can tell it from the built-in one.
fn tiny_registry() -> String {
    r#"{ "schema": 1, "runtimes": { "php": { "displayName": "PHP",
        "versions": [{ "version": "9.9.9", "artifacts": [] }] } } }"#
        .to_owned()
}

fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/registry.json", listener.local_addr().unwrap());
    drop(listener);
    url
}

fn today() -> CalDate {
    CalDate::new(2026, 11, 3).unwrap()
}

#[test]
fn it_starts_with_the_built_in_list_and_no_complaint() {
    let holder = RegistryHolder::with_bundled().unwrap();

    let status = holder.status();
    assert_eq!(status.source, RegistrySourceKind::Bundled);
    assert_eq!(status.age_seconds, None);
    assert!(status.problem.is_none() && !status.refreshing);

    let available = holder.available(Os::Windows, Arch::X86_64, today());
    let ids: Vec<&str> = available.runtimes.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        ["caddy", "composer", "mariadb", "mysql", "node", "php"]
    );
    assert_eq!(available.registry, status);
    assert!(
        available
            .runtimes
            .iter()
            .any(|r| r.versions.iter().any(|v| v.available)),
        "something can be installed on Windows"
    );
}

#[tokio::test]
async fn a_refresh_replaces_the_list_and_says_it_is_fresh() {
    let (_tmp, home) = home();
    let server =
        Server::start(|_, _, stream| respond(stream, "200 OK", &[], tiny_registry().as_bytes()));
    let holder = RegistryHolder::with_bundled().unwrap();

    let status = holder
        .refresh(&home, &downloader(), &server.url("/registry.json"))
        .await
        .unwrap();

    assert_eq!(status.source, RegistrySourceKind::Fresh);
    assert!(status.problem.is_none() && !status.refreshing);
    assert_eq!(holder.status(), status);
    let available = holder.available(Os::Windows, Arch::X86_64, today());
    assert_eq!(available.runtimes.len(), 1);
    assert_eq!(available.runtimes[0].versions[0].version, "9.9.9");
    assert!(
        holder.registry().runtime("node").is_none(),
        "the built-in list is gone"
    );
}

#[tokio::test]
async fn offline_with_nothing_saved_stays_on_the_built_in_list_and_says_why() {
    let (_tmp, home) = home();
    let holder = RegistryHolder::with_bundled().unwrap();

    let status = holder
        .refresh(&home, &downloader(), &dead_url())
        .await
        .unwrap();

    assert_eq!(status.source, RegistrySourceKind::Bundled);
    assert_eq!(status.problem.unwrap().code, ErrorCode::Network);
    assert!(holder.registry().runtime("node").is_some());
}

#[tokio::test]
async fn offline_after_a_good_run_uses_the_saved_copy_with_its_age() {
    let (_tmp, home) = home();
    let server =
        Server::start(|_, _, stream| respond(stream, "200 OK", &[], tiny_registry().as_bytes()));
    RegistryHolder::with_bundled()
        .unwrap()
        .refresh(&home, &downloader(), &server.url("/registry.json"))
        .await
        .unwrap();

    // A new run of the app, no network.
    let holder = RegistryHolder::with_bundled().unwrap();
    let status = holder
        .refresh(&home, &downloader(), &dead_url())
        .await
        .unwrap();

    assert_eq!(status.source, RegistrySourceKind::Cache);
    assert!(status.age_seconds.is_some());
    assert_eq!(status.problem.unwrap().code, ErrorCode::Network);
    assert_eq!(
        holder.registry().runtime("php").unwrap().releases[0]
            .version
            .as_str(),
        "9.9.9"
    );
}

#[tokio::test]
async fn a_list_for_a_newer_lunarun_keeps_the_old_one_and_asks_for_an_update() {
    let (_tmp, home) = home();
    let server = Server::start(|_, _, stream| {
        respond(
            stream,
            "200 OK",
            &[],
            br#"{ "schema": 2, "runtimes": ["a new shape"] }"#,
        );
    });
    let holder = RegistryHolder::with_bundled().unwrap();

    let status = holder
        .refresh(&home, &downloader(), &server.url("/registry.json"))
        .await
        .unwrap();

    assert_eq!(status.source, RegistrySourceKind::Bundled);
    assert_eq!(
        status.problem.unwrap().code,
        ErrorCode::RegistryUnsupportedSchema
    );
    assert!(holder.registry().runtime("node").is_some());
}

#[tokio::test]
async fn a_refresh_that_is_already_running_is_not_started_twice() {
    let (_tmp, home) = home();
    let server = Server::start(|_, _, stream| {
        std::thread::sleep(Duration::from_millis(400));
        respond(stream, "200 OK", &[], tiny_registry().as_bytes());
    });
    let url = server.url("/registry.json");
    let holder = RegistryHolder::with_bundled().unwrap();
    let downloader = downloader();

    let first = holder.refresh(&home, &downloader, &url);
    let second = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let during = holder.status();
        let joined = holder.refresh(&home, &downloader, &url).await.unwrap();
        (during, joined)
    };
    let (first, (during, joined)) = tokio::join!(first, second);

    assert!(
        during.refreshing,
        "the page can show that a refresh is running"
    );
    assert!(
        joined.refreshing,
        "the second caller got the status of the running one"
    );
    assert_eq!(server.request_count(), 1, "one download, not two");
    assert_eq!(first.unwrap().source, RegistrySourceKind::Fresh);
    assert!(!holder.status().refreshing, "cleared when it ends");
}

#[tokio::test]
async fn a_second_refresh_after_the_first_ended_runs_again() {
    let (_tmp, home) = home();
    let server =
        Server::start(|_, _, stream| respond(stream, "200 OK", &[], tiny_registry().as_bytes()));
    let url = server.url("/registry.json");
    let holder = RegistryHolder::with_bundled().unwrap();

    holder.refresh(&home, &downloader(), &url).await.unwrap();
    holder.refresh(&home, &downloader(), &url).await.unwrap();

    assert_eq!(server.request_count(), 2);
}

#[test]
fn the_page_gets_camel_case_json_with_lowercase_source_names() {
    let holder = RegistryHolder::with_bundled().unwrap();
    let value = serde_json::to_value(holder.available(Os::Windows, Arch::X86_64, today())).unwrap();

    assert_eq!(value["registry"]["source"], "bundled");
    assert!(value["registry"]["ageSeconds"].is_null());
    assert_eq!(value["registry"]["refreshing"], false);
    assert!(value["registry"]["problem"].is_null());
    assert_eq!(value["runtimes"][0]["id"], "caddy");
    assert!(value["runtimes"][0]["displayName"].is_string());
}
