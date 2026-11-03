//! Installed runtimes, adapter layouts and the default `verify`. Every data folder has a space and
//! non-ASCII characters in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::error::ErrorCode;
use luna_core::runtimes::{
    Binary, InstallReceipt, RECEIPT_FILE, RuntimeAdapter, VersionCommand, adapter_for, adapters,
    list_installed, run_version_command,
};
use serde_json::json;
use tempfile::TempDir;

fn home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

/// Create `runtimes/<runtime>/<version>/` with a valid receipt.
fn install(home: &LunaHome, runtime: &str, version: &str) -> PathBuf {
    let dir = home.runtime_dir(runtime, version).unwrap();
    fs::create_dir_all(&dir).unwrap();
    InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[])
        .write(&dir)
        .unwrap();
    dir
}

fn listed(home: &LunaHome) -> Vec<String> {
    list_installed(home)
        .unwrap()
        .iter()
        .map(|i| format!("{} {}", i.runtime, i.version))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Installed runtimes
// ---------------------------------------------------------------------------------------------

#[test]
fn nothing_is_installed_in_a_fresh_home() {
    let (_tmp, home) = home();
    assert!(list_installed(&home).unwrap().is_empty());
    home.ensure_layout().unwrap();
    assert!(list_installed(&home).unwrap().is_empty());
}

#[test]
fn installed_runtimes_are_listed_by_name_and_newest_version_first() {
    let (_tmp, home) = home();
    for (runtime, version) in [
        ("php", "8.3.14"),
        ("php", "8.10.0"),
        ("php", "8.9.0"),
        ("node", "24.22.2"),
        ("mysql", "8.4.13"),
        ("php", "8.5.0-rc1"),
    ] {
        install(&home, runtime, version);
    }
    assert_eq!(
        listed(&home),
        [
            "mysql 8.4.13",
            "node 24.22.2",
            "php 8.10.0",
            "php 8.9.0",
            "php 8.5.0-rc1",
            "php 8.3.14",
        ],
        "8.10 is newer than 8.9, a release candidate sorts below its release"
    );
}

#[test]
fn the_listing_carries_the_receipt_and_the_folder() {
    let (_tmp, home) = home();
    let dir = install(&home, "node", "24.22.2");
    let found = list_installed(&home).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dir, dir);
    assert_eq!(found[0].receipt.runtime, "node");
    assert_eq!(found[0].receipt.os, "windows");
    assert_eq!(found[0].receipt.arch, "x86_64");
    assert!(
        found[0].receipt.installed_at > 1_700_000_000,
        "a real timestamp"
    );
}

#[test]
fn work_folders_and_folders_without_a_trustworthy_receipt_are_not_installed_runtimes() {
    let (_tmp, home) = home();
    install(&home, "php", "8.3.14");
    let php = home.runtimes_dir().join("php");

    // The installer's staging folders, hidden folders.
    fs::create_dir_all(php.join(".staging-123")).unwrap();
    InstallReceipt::new("php", "8.4.0", Os::Windows, Arch::X86_64, false, &[])
        .write(&php.join(".staging-123"))
        .unwrap();

    // No receipt: an extraction that never finished.
    fs::create_dir_all(php.join("8.4.1")).unwrap();
    fs::write(php.join("8.4.1").join("php.exe"), "x").unwrap();

    // A damaged receipt, and one from a lunaRun that is newer than this one.
    fs::create_dir_all(php.join("8.4.2")).unwrap();
    fs::write(php.join("8.4.2").join(RECEIPT_FILE), "{ not json").unwrap();
    fs::create_dir_all(php.join("8.4.3")).unwrap();
    fs::write(
        php.join("8.4.3").join(RECEIPT_FILE),
        json!({ "schema": 99, "runtime": "php", "version": "8.4.3" }).to_string(),
    )
    .unwrap();
    // No schema field at all.
    fs::create_dir_all(php.join("8.4.4")).unwrap();
    fs::write(
        php.join("8.4.4").join(RECEIPT_FILE),
        json!({ "runtime": "php", "version": "8.4.4", "os": "windows", "arch": "x86_64",
                "installedAt": 1, "artifacts": [] })
        .to_string(),
    )
    .unwrap();

    // A copied folder: its receipt names a different version than the folder.
    let copied = php.join("8.5.0");
    fs::create_dir_all(&copied).unwrap();
    fs::copy(
        php.join("8.3.14").join(RECEIPT_FILE),
        copied.join(RECEIPT_FILE),
    )
    .unwrap();

    // A complete, valid receipt from a lunaRun that is newer than this one, and one with schema 0:
    // only the schema number is wrong, so nothing but the schema check can reject them.
    for (version, schema) in [("8.6.0", 2), ("8.6.1", 0)] {
        let dir = install(&home, "php", version);
        let path = dir.join(RECEIPT_FILE);
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        value["schema"] = json!(schema);
        fs::write(&path, value.to_string()).unwrap();
    }

    // A hidden runtime folder (the installer's own scratch space) whose receipt looks fine.
    install(&home, ".trash", "8.3.14");

    // A folder that is not a version at all, and a plain file where a folder belongs.
    fs::create_dir_all(php.join("latest")).unwrap();
    fs::write(php.join("notes.txt"), "hello").unwrap();
    fs::write(home.runtimes_dir().join("stray.txt"), "hello").unwrap();

    assert_eq!(listed(&home), ["php 8.3.14"]);
}

#[test]
fn listing_never_touches_a_damaged_receipt() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("php", "8.4.2").unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(RECEIPT_FILE), "{ not json").unwrap();

    assert!(list_installed(&home).unwrap().is_empty());

    // Read-only: the file is still there, not moved aside or rewritten.
    assert_eq!(
        fs::read_to_string(dir.join(RECEIPT_FILE)).unwrap(),
        "{ not json"
    );
    let names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [RECEIPT_FILE]);
}

#[cfg(unix)]
#[test]
fn a_symlink_where_a_runtime_folder_belongs_is_not_followed() {
    let (tmp, home) = home();
    install(&home, "php", "8.3.14");
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    InstallReceipt::new("php", "9.9.9", Os::Linux, Arch::X86_64, false, &[])
        .write(&elsewhere)
        .unwrap();
    std::os::unix::fs::symlink(&elsewhere, home.runtimes_dir().join("php").join("9.9.9")).unwrap();

    assert_eq!(listed(&home), ["php 8.3.14"]);
}

#[test]
fn a_receipt_round_trips_with_every_artifact() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("php", "8.3.32").unwrap();
    fs::create_dir_all(&dir).unwrap();
    let mut receipt = InstallReceipt::new("php", "8.3.32", Os::Macos, Arch::Aarch64, true, &[]);
    receipt.artifacts = vec![
        luna_core::runtimes::ReceiptArtifact {
            variant: Some("static-cli".to_owned()),
            url: "https://example.com/php-cli.tar.gz".to_owned(),
            sha256: "a".repeat(64),
            size: 13_688_227,
        },
        luna_core::runtimes::ReceiptArtifact {
            variant: Some("static-fpm".to_owned()),
            url: "https://example.com/php-fpm.tar.gz".to_owned(),
            sha256: "b".repeat(64),
            size: 13_711_559,
        },
    ];
    receipt.write(&dir).unwrap();

    let found = &list_installed(&home).unwrap()[0];
    assert_eq!(found.receipt, receipt);
    assert!(found.receipt.rosetta);
    let text = fs::read_to_string(dir.join(RECEIPT_FILE)).unwrap();
    assert!(text.contains("\"schema\": 1"));
}

// ---------------------------------------------------------------------------------------------
// artifacts_for
// ---------------------------------------------------------------------------------------------

mod artifacts {
    use luna_core::registry::{CalDate, Registry};
    use serde_json::{Value, json};

    use super::*;

    fn artifact(os: &str, arch: &str, variant: Option<&str>, size: u64) -> Value {
        let mut value = json!({
            "os": os, "arch": arch, "format": "tar.gz",
            "url": format!("https://example.com/{os}-{arch}-{}-{size}.tar.gz", variant.unwrap_or("x")),
            "sha256": "0".repeat(64), "size": size,
        });
        if let Some(variant) = variant {
            value["variant"] = json!(variant);
        }
        value
    }

    fn registry() -> Registry {
        let text = json!({
            "schema": 1,
            "runtimes": { "php": { "displayName": "PHP", "versions": [
                { "version": "8.3.32", "artifacts": [
                    artifact("macos", "aarch64", Some("static-cli"), 100),
                    artifact("macos", "aarch64", Some("static-fpm"), 150),
                    artifact("macos", "x86_64", Some("static-cli"), 110),
                    artifact("macos", "x86_64", Some("static-fpm"), 160),
                    artifact("windows", "x86_64", Some("nts-vs16"), 900),
                ] },
                { "version": "8.2.1", "artifacts": [
                    artifact("macos", "x86_64", Some("static-cli"), 70),
                    artifact("macos", "x86_64", Some("static-fpm"), 80),
                ] },
            ] } }
        })
        .to_string();
        Registry::parse(&text).unwrap()
    }

    #[test]
    fn every_part_of_an_install_is_returned_with_the_total_size() {
        let registry = registry();
        let mac = registry
            .artifacts_for("php", "8.3.32", Os::Macos, Arch::Aarch64)
            .unwrap();
        let variants: Vec<_> = mac
            .artifacts
            .iter()
            .map(|a| a.variant.as_deref().unwrap())
            .collect();
        assert_eq!(variants, ["static-cli", "static-fpm"]);
        assert_eq!(mac.total_size(), 250);
        assert!(!mac.rosetta_fallback);

        let windows = registry
            .artifacts_for("php", "8.3.32", Os::Windows, Arch::X86_64)
            .unwrap();
        assert_eq!(windows.artifacts.len(), 1);
        assert_eq!(windows.total_size(), 900);
    }

    #[test]
    fn apple_silicon_falls_back_to_the_whole_intel_set_only_when_it_has_no_native_one() {
        let registry = registry();
        let fallback = registry
            .artifacts_for("php", "8.2.1", Os::Macos, Arch::Aarch64)
            .unwrap();
        assert!(fallback.rosetta_fallback);
        assert_eq!(fallback.artifacts.len(), 2, "both halves, never just one");
        assert!(fallback.artifacts.iter().all(|a| a.arch == Arch::X86_64));
        assert_eq!(fallback.total_size(), 150);
    }

    #[test]
    fn missing_versions_and_platforms_are_typed_errors() {
        let registry = registry();
        let no_platform = registry
            .artifacts_for("php", "8.2.1", Os::Linux, Arch::X86_64)
            .unwrap_err();
        assert_eq!(no_platform.code, ErrorCode::NoArtifactForPlatform);
        let windows_arm = registry
            .artifacts_for("php", "8.3.32", Os::Windows, Arch::Aarch64)
            .unwrap_err();
        assert_eq!(windows_arm.code, ErrorCode::NoArtifactForPlatform);
        for (runtime, version) in [("ruby", "3.3.0"), ("php", "9.9.9")] {
            let err = registry
                .artifacts_for(runtime, version, Os::Macos, Arch::Aarch64)
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound, "{runtime} {version}");
        }
    }

    #[test]
    fn the_ui_rows_show_the_total_download_not_just_the_first_archive() {
        let registry = registry();
        let today = CalDate::new(2026, 11, 3).unwrap();
        let rows = registry.available_versions(Os::Macos, Arch::Aarch64, today);
        let row = rows.iter().find(|r| r.version == "8.3.32").unwrap();
        assert_eq!(row.size, Some(250));
        let fallback = rows.iter().find(|r| r.version == "8.2.1").unwrap();
        assert_eq!(fallback.size, Some(150));
        assert!(fallback.rosetta_fallback);
    }
}

// ---------------------------------------------------------------------------------------------
// Adapter layouts
// ---------------------------------------------------------------------------------------------

fn rel(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap()
        .components()
        .map(|c| c.as_os_str().to_str().unwrap())
        .collect::<Vec<_>>()
        .join("/")
}

fn layout(adapter: &dyn RuntimeAdapter, os: Os) -> Vec<(String, String, bool)> {
    let dir = Path::new("/data/lüna home/runtimes/x/1.2.3");
    adapter
        .binaries(dir, os)
        .into_iter()
        .map(|b| (b.name.to_owned(), rel(dir, &b.path), b.required))
        .collect()
}

fn path_dirs(adapter: &dyn RuntimeAdapter, os: Os) -> Vec<String> {
    let dir = Path::new("/data/lüna home/runtimes/x/1.2.3");
    adapter
        .path_dirs(dir, os)
        .iter()
        .map(|p| {
            let shown = rel(dir, p);
            if shown.is_empty() {
                ".".to_owned()
            } else {
                shown
            }
        })
        .collect()
}

fn entry(name: &str, path: &str, required: bool) -> (String, String, bool) {
    (name.to_owned(), path.to_owned(), required)
}

#[test]
fn there_is_one_adapter_per_registry_runtime() {
    let ids: Vec<_> = adapters().iter().map(|a| a.id()).collect();
    assert_eq!(
        ids,
        ["caddy", "composer", "mariadb", "mysql", "node", "php"]
    );
    for id in &ids {
        assert_eq!(adapter_for(id).unwrap().id(), *id);
    }
    assert!(adapter_for("ruby").is_none());
    assert!(adapter_for("").is_none());

    // Every runtime the generated registry offers can be installed by some adapter.
    let registry =
        luna_core::registry::Registry::parse(include_str!("../../../registry/registry.json"))
            .unwrap();
    for id in registry.runtime_ids() {
        assert!(adapter_for(id).is_some(), "no adapter for {id}");
    }
}

#[test]
fn php_layouts() {
    let php = adapter_for("php").unwrap();
    assert_eq!(
        layout(php, Os::Windows),
        [
            entry("php", "php.exe", true),
            entry("php-cgi", "php-cgi.exe", true)
        ]
    );
    for os in [Os::Macos, Os::Linux] {
        assert_eq!(
            layout(php, os),
            [entry("php", "php", true), entry("php-fpm", "php-fpm", true)],
            "static builds extract flat: {os:?}"
        );
        assert_eq!(path_dirs(php, os), ["."]);
    }
    assert_eq!(path_dirs(php, Os::Windows), ["."]);
}

#[test]
fn node_layouts() {
    let node = adapter_for("node").unwrap();
    assert_eq!(
        layout(node, Os::Windows),
        [
            entry("node", "node.exe", true),
            entry("npm", "npm.cmd", false),
            entry("npx", "npx.cmd", false)
        ]
    );
    assert_eq!(path_dirs(node, Os::Windows), ["."]);
    for os in [Os::Macos, Os::Linux] {
        assert_eq!(
            layout(node, os),
            [
                entry("node", "bin/node", true),
                entry("npm", "bin/npm", false),
                entry("npx", "bin/npx", false)
            ]
        );
        assert_eq!(path_dirs(node, os), ["bin"]);
    }
}

#[test]
fn database_layouts() {
    let mysql = adapter_for("mysql").unwrap();
    assert_eq!(
        layout(mysql, Os::Windows),
        [
            entry("mysqld", "bin/mysqld.exe", true),
            entry("mysql", "bin/mysql.exe", false),
            entry("mysqladmin", "bin/mysqladmin.exe", false)
        ]
    );
    assert_eq!(
        layout(mysql, Os::Linux)[0],
        entry("mysqld", "bin/mysqld", true)
    );
    assert_eq!(path_dirs(mysql, Os::Macos), ["bin"]);

    let mariadb = adapter_for("mariadb").unwrap();
    assert_eq!(
        layout(mariadb, Os::Windows),
        [
            entry("mariadbd", "bin/mariadbd.exe", true),
            entry("mariadb", "bin/mariadb.exe", false),
            entry("mariadb-admin", "bin/mariadb-admin.exe", false),
            entry("mariadb-install-db", "bin/mariadb-install-db.exe", false)
        ]
    );
    assert_eq!(
        layout(mariadb, Os::Linux)[0],
        entry("mariadbd", "bin/mariadbd", true)
    );
}

#[test]
fn caddy_and_composer_layouts() {
    let caddy = adapter_for("caddy").unwrap();
    assert_eq!(
        layout(caddy, Os::Windows),
        [entry("caddy", "caddy.exe", true)]
    );
    assert_eq!(layout(caddy, Os::Macos), [entry("caddy", "caddy", true)]);
    let composer = adapter_for("composer").unwrap();
    for os in [Os::Windows, Os::Macos, Os::Linux] {
        assert_eq!(
            layout(composer, os),
            [entry("composer", "composer.phar", true)]
        );
    }
}

#[test]
fn every_version_command_runs_a_program_inside_the_install_folder() {
    let dir = Path::new("/data/lüna home/runtimes/x/1.2.3");
    for adapter in adapters() {
        for os in [Os::Windows, Os::Macos, Os::Linux] {
            match adapter.version_command(dir, os) {
                Some(VersionCommand { program, args }) => {
                    assert!(program.starts_with(dir), "{} {os:?}", adapter.id());
                    assert!(!args.is_empty());
                }
                None => assert_eq!(adapter.id(), "composer"),
            }
        }
    }
}

#[test]
fn versions_are_read_from_what_each_runtime_prints() {
    let parse = |id: &str, output: &str| adapter_for(id).unwrap().parse_version(output);

    assert_eq!(
        parse("php", "PHP 8.3.36 (cli) (built: Oct 21 2026 08:10:22) (NTS Visual C++ 2019 x64)\nCopyright (c) The PHP Group").as_deref(),
        Some("8.3.36")
    );
    assert_eq!(
        parse("php", "PHP 8.5.0 (cli)").as_deref(),
        Some("8.5.0"),
        "a pre-release prints its numbers only"
    );
    assert_eq!(parse("node", "v24.22.2\n").as_deref(), Some("24.22.2"));
    assert_eq!(parse("node", "v24.22.2\r\n").as_deref(), Some("24.22.2"));
    assert_eq!(
        parse(
            "mysql",
            "C:\\x\\bin\\mysqld.exe  Ver 8.4.13 for Win64 on x86_64 (MySQL Community Server - GPL)"
        )
        .as_deref(),
        Some("8.4.13")
    );
    assert_eq!(
        parse(
            "mariadb",
            "mariadbd.exe  Ver 11.8.9-MariaDB for Win64 on AMD64 (mariadb.org binary distribution)"
        )
        .as_deref(),
        Some("11.8.9")
    );
    assert_eq!(
        parse("caddy", "v2.11.7 h1:abcdef=\n").as_deref(),
        Some("2.11.7")
    );

    for id in ["php", "node", "mysql", "mariadb", "caddy"] {
        assert_eq!(parse(id, ""), None, "{id}");
        assert_eq!(parse(id, "command not found"), None, "{id}");
    }
}

// ---------------------------------------------------------------------------------------------
// The default verify, through a test adapter that runs a real script
// ---------------------------------------------------------------------------------------------

/// A runtime whose "binary" is a script that prints whatever the test wants.
struct Scripted;

impl Scripted {
    fn script_name(os: Os) -> &'static str {
        if os == Os::Windows {
            "tool.cmd"
        } else {
            "tool.sh"
        }
    }
}

impl RuntimeAdapter for Scripted {
    fn id(&self) -> &'static str {
        "scripted"
    }
    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary> {
        vec![Binary {
            name: "tool",
            path: dir.join(Self::script_name(os)),
            required: true,
        }]
    }
    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.to_path_buf()]
    }
    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand> {
        Some(VersionCommand {
            program: dir.join(Self::script_name(os)),
            args: vec!["--version"],
        })
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output.split_whitespace().nth(1).map(str::to_owned)
    }
}

fn current_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// Write a script that runs `body` (a line of shell, or of cmd on Windows).
fn script(dir: &Path, body: &str) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(Scripted::script_name(current_os()));
    if cfg!(windows) {
        fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
    } else {
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

fn sleep_command(seconds: u32) -> String {
    if cfg!(windows) {
        format!("ping -n {} 127.0.0.1 >nul", seconds + 1)
    } else {
        format!("sleep {seconds}")
    }
}

fn run_verify(
    dir: &Path,
    expected: &str,
) -> Result<luna_core::runtimes::VersionInfo, luna_core::LunaError> {
    Scripted.verify(dir, current_os(), expected)
}

#[test]
fn verify_accepts_a_runtime_that_reports_the_installed_version() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("scripted", "1.2.3").unwrap();
    script(&dir, "echo tool 1.2.3 built-yesterday");
    let info = run_verify(&dir, "1.2.3").unwrap();
    assert_eq!(info.version, "1.2.3");
    assert!(info.output.contains("built-yesterday"));
}

#[test]
fn verify_rejects_a_missing_binary_a_wrong_version_and_unreadable_output() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("scripted", "1.2.3").unwrap();

    fs::create_dir_all(&dir).unwrap();
    let missing = run_verify(&dir, "1.2.3").unwrap_err();
    assert_eq!(missing.code, ErrorCode::RuntimeBroken);
    assert!(
        missing.message.contains("is missing"),
        "{}",
        missing.message
    );
    assert_eq!(missing.details.unwrap()["runtime"], "scripted");

    script(&dir, "echo tool 1.2.4");
    let wrong = run_verify(&dir, "1.2.3").unwrap_err();
    assert_eq!(wrong.code, ErrorCode::RuntimeBroken);
    assert!(
        wrong
            .message
            .contains("reports version 1.2.4 but 1.2.3 was installed")
    );

    script(&dir, "echo tool");
    let unreadable = run_verify(&dir, "1.2.3").unwrap_err();
    assert_eq!(unreadable.code, ErrorCode::RuntimeBroken);
    assert!(unreadable.message.contains("could not read a version"));
}

#[test]
fn verify_rejects_a_program_that_fails() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("scripted", "1.2.3").unwrap();
    script(&dir, "echo tool 1.2.3\nexit 3");
    let err = run_verify(&dir, "1.2.3").unwrap_err();
    assert_eq!(err.code, ErrorCode::RuntimeBroken);
    assert!(err.message.contains("exited with"), "{}", err.message);
}

#[test]
fn a_program_that_hangs_is_killed_and_reported() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("scripted", "1.2.3").unwrap();
    script(&dir, &sleep_command(20));
    let command = Scripted.version_command(&dir, current_os()).unwrap();

    let started = std::time::Instant::now();
    let err =
        run_version_command("scripted", &dir, &command, Duration::from_millis(400)).unwrap_err();

    assert_eq!(err.code, ErrorCode::RuntimeBroken);
    assert!(err.message.contains("did not finish"), "{}", err.message);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "it was killed, not waited for"
    );
}

#[cfg(unix)]
#[test]
fn verify_rejects_a_binary_without_the_execute_bit() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, home) = home();
    let dir = home.runtime_dir("scripted", "1.2.3").unwrap();
    script(&dir, "echo tool 1.2.3");
    let path = dir.join("tool.sh");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let err = run_verify(&dir, "1.2.3").unwrap_err();
    assert!(err.message.contains("not executable"), "{}", err.message);
}

#[test]
fn composer_is_checked_by_its_content_because_it_cannot_run_without_php() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("composer", "2.11.1").unwrap();
    fs::create_dir_all(&dir).unwrap();
    let composer = adapter_for("composer").unwrap();
    let phar = dir.join("composer.phar");

    let missing = composer.verify(&dir, current_os(), "2.11.1").unwrap_err();
    assert_eq!(missing.code, ErrorCode::RuntimeBroken);

    // An error page saved under the right name.
    fs::write(&phar, "<html>404</html>").unwrap();
    let tiny = composer.verify(&dir, current_os(), "2.11.1").unwrap_err();
    assert!(tiny.message.contains("bytes"), "{}", tiny.message);

    // Big enough but not a PHP archive.
    fs::write(&phar, vec![b'x'; 600 * 1024]).unwrap();
    let not_php = composer.verify(&dir, current_os(), "2.11.1").unwrap_err();
    assert!(
        not_php.message.contains("not a PHP archive"),
        "{}",
        not_php.message
    );

    let mut real = b"#!/usr/bin/env php\n<?php\n".to_vec();
    real.resize(600 * 1024, b' ');
    fs::write(&phar, real).unwrap();
    let info = composer.verify(&dir, current_os(), "2.11.1").unwrap();
    assert_eq!(info.version, "2.11.1");
    assert!(info.output.contains("not run"));
}

#[test]
fn a_pre_release_install_verifies_against_the_numbers_its_binary_prints() {
    let (_tmp, home) = home();
    let dir = home.runtime_dir("scripted", "1.2.3-rc1").unwrap();
    script(&dir, "echo tool 1.2.3");
    assert_eq!(run_verify(&dir, "1.2.3-rc1").unwrap().version, "1.2.3");
    assert!(run_verify(&dir, "1.2.4-rc1").is_err());
}

#[test]
fn the_installed_view_is_plain_data_for_the_page() {
    use luna_core::runtimes::{InstalledView, ReceiptArtifact};
    let (_tmp, home) = home();
    let dir = home.runtime_dir("php", "8.3.32").unwrap();
    fs::create_dir_all(&dir).unwrap();
    let mut receipt = InstallReceipt::new("php", "8.3.32", Os::Macos, Arch::Aarch64, true, &[]);
    receipt.artifacts = ["static-cli", "static-fpm"]
        .iter()
        .map(|variant| ReceiptArtifact {
            variant: Some((*variant).to_owned()),
            url: format!("https://example.com/{variant}.tar.gz"),
            sha256: "a".repeat(64),
            size: 10,
        })
        .collect();
    receipt.write(&dir).unwrap();

    let installed = list_installed(&home).unwrap();
    let view = InstalledView::from(&installed[0]);

    assert_eq!(
        (view.runtime.as_str(), view.version.as_str()),
        ("php", "8.3.32")
    );
    assert_eq!(view.dir, dir.display().to_string());
    assert!(view.rosetta);
    assert_eq!(view.parts, 2, "PHP on macOS is two downloads");
    assert!(view.installed_at > 1_700_000_000);
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["installedAt"], view.installed_at);
    assert_eq!(json["parts"], 2);
}
