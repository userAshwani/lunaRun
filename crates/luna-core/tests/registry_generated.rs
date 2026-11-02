//! The registry file the generator wrote (`registry/registry.json`) must be accepted by the app's own
//! parser, so the two can never drift apart. A change to the generator or to the parser that breaks this
//! fails here, in CI, instead of in a user's Runtimes page.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use luna_core::diagnostics::{Arch, Os};
use luna_core::registry::{CalDate, Registry};

const GENERATED: &str = include_str!("../../../registry/registry.json");

fn registry() -> Registry {
    Registry::parse(GENERATED).expect("registry/registry.json is rejected by the app's parser")
}

#[test]
fn the_generated_registry_is_accepted_by_the_parser() {
    let registry = registry();
    let ids: Vec<&str> = registry.runtime_ids().collect();
    assert_eq!(
        ids,
        ["caddy", "composer", "mariadb", "mysql", "node", "php"]
    );
    for id in ids {
        let runtime = registry.runtime(id).unwrap();
        assert!(!runtime.releases.is_empty(), "{id} has no versions");
        assert!(
            runtime.releases.iter().all(|r| !r.artifacts.is_empty()),
            "{id}: a version without any download"
        );
    }
}

#[test]
fn every_download_is_usable_as_written() {
    // Nothing the generator wrote may be silently skipped by the parser (an unknown OS or format would be).
    let registry = registry();
    let json: serde_json::Value = serde_json::from_str(GENERATED).unwrap();
    for (id, runtime) in json["runtimes"].as_object().unwrap() {
        let written: usize = runtime["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["artifacts"].as_array().unwrap().len())
            .sum();
        let parsed: usize = registry
            .runtime(id)
            .unwrap()
            .releases
            .iter()
            .map(|r| r.artifacts.len())
            .sum();
        assert_eq!(parsed, written, "{id}: the parser skipped some downloads");
    }
}

#[test]
fn a_windows_pc_can_install_php_and_node_and_lines_are_flagged() {
    let registry = registry();
    let today = CalDate::new(2026, 11, 2).unwrap();
    let rows = registry.available_versions(Os::Windows, Arch::X86_64, today);

    let php: Vec<_> = rows.iter().filter(|r| r.runtime == "php").collect();
    assert!(
        php.iter().all(|r| r.available),
        "every PHP line has a Windows build"
    );
    assert!(
        php.iter().any(|r| r.line == "7.4" && r.eol),
        "PHP 7.4 is end of life"
    );
    assert!(
        php.iter().any(|r| !r.eol),
        "at least one PHP line is still supported"
    );

    let node = rows
        .iter()
        .filter(|r| r.runtime == "node")
        .collect::<Vec<_>>();
    assert!(
        node.iter()
            .all(|r| r.available && r.size.unwrap() > 1_000_000)
    );
}

#[test]
fn macos_and_linux_get_node_but_not_yet_php() {
    // PHP for macOS and Linux needs its own source (docs/PRD.md Q3). Say so in a test, so the day it
    // arrives this one fails and gets updated on purpose.
    let registry = registry();
    let today = CalDate::new(2026, 11, 2).unwrap();
    for (os, arch) in [
        (Os::Macos, Arch::Aarch64),
        (Os::Macos, Arch::X86_64),
        (Os::Linux, Arch::X86_64),
        (Os::Linux, Arch::Aarch64),
    ] {
        let rows = registry.available_versions(os, arch, today);
        assert!(
            rows.iter().any(|r| r.runtime == "node" && r.available),
            "{os:?}/{arch:?}"
        );
        assert!(
            !rows.iter().any(|r| r.runtime == "php" && r.available),
            "{os:?}/{arch:?}"
        );
    }
}

#[test]
fn caddy_and_composer_are_offered_on_every_platform() {
    let registry = registry();
    let today = CalDate::new(2026, 11, 2).unwrap();
    for (os, arch) in [
        (Os::Windows, Arch::X86_64),
        (Os::Macos, Arch::Aarch64),
        (Os::Macos, Arch::X86_64),
        (Os::Linux, Arch::X86_64),
        (Os::Linux, Arch::Aarch64),
    ] {
        let rows = registry.available_versions(os, arch, today);
        for runtime in ["caddy", "composer"] {
            assert!(
                rows.iter().any(|r| r.runtime == runtime && r.available),
                "{runtime} on {os:?}/{arch:?}"
            );
        }
    }
}

#[test]
fn composer_is_a_single_file_and_caddy_is_an_archive() {
    use luna_core::registry::ArtifactFormat;
    let registry = registry();
    let composer = registry
        .artifact_for("composer", "2.11.1", Os::Linux, Arch::X86_64)
        .unwrap();
    assert_eq!(composer.artifact.format, ArtifactFormat::File);
    assert!(composer.artifact.url.ends_with("/composer.phar"));

    let caddy = &registry.runtime("caddy").unwrap().releases[0];
    let windows = registry
        .artifact_for("caddy", caddy.version.as_str(), Os::Windows, Arch::X86_64)
        .unwrap();
    assert_eq!(windows.artifact.format, ArtifactFormat::Zip);
}

#[test]
fn mariadb_is_offered_on_windows_and_linux_x64_only() {
    // MariaDB publishes no macOS binaries and no Linux arm64 ones (docs/RUNTIMES.md section 2).
    let registry = registry();
    let today = CalDate::new(2026, 11, 2).unwrap();
    let offered = |os, arch| {
        registry
            .available_versions(os, arch, today)
            .iter()
            .any(|r| r.runtime == "mariadb" && r.available)
    };
    assert!(offered(Os::Windows, Arch::X86_64));
    assert!(offered(Os::Linux, Arch::X86_64));
    assert!(!offered(Os::Linux, Arch::Aarch64));
    // An Apple silicon Mac must not get an Intel Linux or Windows build by mistake.
    assert!(!offered(Os::Macos, Arch::Aarch64));
    assert!(!offered(Os::Macos, Arch::X86_64));

    let lines = &registry.runtime("mariadb").unwrap().lines;
    assert!(
        lines.contains_key("10.6"),
        "the end-of-life line is still listed"
    );
}

#[test]
fn mysql_is_offered_on_windows_macos_and_linux_x64_and_flags_the_old_line() {
    // Linux arm64 is left out on purpose (docs/DECISIONS.md D-017).
    let registry = registry();
    let today = CalDate::new(2026, 11, 2).unwrap();
    let offered = |os, arch| {
        registry
            .available_versions(os, arch, today)
            .iter()
            .any(|r| r.runtime == "mysql" && r.available)
    };
    assert!(offered(Os::Windows, Arch::X86_64));
    assert!(offered(Os::Macos, Arch::Aarch64));
    assert!(offered(Os::Macos, Arch::X86_64));
    assert!(offered(Os::Linux, Arch::X86_64));
    assert!(!offered(Os::Linux, Arch::Aarch64));

    let rows = registry.available_versions(Os::Windows, Arch::X86_64, today);
    let old = rows
        .iter()
        .find(|r| r.runtime == "mysql" && r.line == "8.0")
        .expect("MySQL 8.0 is listed");
    assert!(old.eol, "MySQL 8.0 ended 2026-04-30");
    assert!(
        rows.iter().any(|r| r.runtime == "mysql" && !r.eol),
        "a supported MySQL line exists"
    );
}
