//! Opt-in checks of the adapter layouts against the **real archives** of this platform. For every runtime in
//! the generated registry, the newest version that has a build for this computer is downloaded (checked
//! against the registry's SHA-256) and extracted the way the registry says.
//!
//! Both tests need internet access and download a few hundred megabytes (MySQL and MariaDB are the big
//! ones), so they are ignored by default:
//!
//!     # Layout only: every file an adapter expects exists. Starts nothing.
//!     cargo test -p luna-core --test runtimes_online real_archives_have_the_layout_the_adapters_expect -- --ignored --nocapture
//!
//!     # Also starts every real binary (`php -v`, `node -v`, `mysqld --version`, ...) and compares the
//!     # version it prints. Runs third-party programs downloaded from the internet, so it is a separate test.
//!     cargo test -p luna-core --test runtimes_online real_binaries_report_the_installed_version -- --ignored --nocapture
//!
//! Each proves only the platform it runs on.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, DownloadRequest, Downloader};
use luna_core::extract::{ExtractOptions, extract_sync};
use luna_core::registry::{ArtifactFormat, Registry};
use luna_core::runtimes::adapter_for;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Move every entry of `from` into `to`, refusing to overwrite anything. How two flat archives, such as
/// PHP's `php` and `php-fpm`, end up in one install folder.
fn merge_into(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        assert!(!target.exists(), "{} exists twice", target.display());
        fs::rename(entry.path(), target).unwrap();
    }
}

struct Installed {
    id: String,
    version: String,
    dir: PathBuf,
    parts: usize,
    megabytes: u64,
}

/// Download and extract the newest build of every runtime for this platform into a throw-away home.
async fn install_everything(home: &LunaHome) -> Vec<Installed> {
    let registry = Registry::parse(include_str!("../../../registry/registry.json")).unwrap();
    let (os, arch) = (Os::current(), Arch::current());
    let downloader = Downloader::new(DownloadOptions::default()).unwrap();
    let cancel = CancellationToken::new();

    let mut installed = Vec::new();
    for id in registry.runtime_ids() {
        let runtime = registry.runtime(id).unwrap();
        let Some((version, set)) = runtime.releases.iter().find_map(|release| {
            registry
                .artifacts_for(id, release.version.as_str(), os, arch)
                .ok()
                .map(|set| (release.version.to_string(), set))
        }) else {
            eprintln!("{id}: no build for {os:?}/{arch:?}, skipped");
            continue;
        };

        let dir = home.runtime_dir(id, &version).unwrap();
        for artifact in &set.artifacts {
            let base = artifact.url.rsplit('/').next().unwrap();
            let archive = downloader
                .download(
                    home,
                    &DownloadRequest {
                        url: &artifact.url,
                        file_name: base,
                        sha256: &artifact.sha256,
                    },
                    &cancel,
                    |_| {},
                )
                .await
                .unwrap();

            match artifact.format.archive_format() {
                None => {
                    assert_eq!(artifact.format, ArtifactFormat::File);
                    fs::create_dir_all(&dir).unwrap();
                    fs::copy(&archive, dir.join(base)).unwrap();
                }
                Some(format) => {
                    // The extractor wants an empty folder, so each archive gets its own and is merged.
                    let part = home
                        .runtimes_dir()
                        .join(id)
                        .join(format!(".staging-{base}"));
                    let options = ExtractOptions {
                        strip_components: artifact.strip_components,
                        ..ExtractOptions::default()
                    };
                    extract_sync(&archive, &part, format, &options, &cancel, &mut |_| {}).unwrap();
                    merge_into(&part, &dir);
                    fs::remove_dir_all(&part).unwrap();
                }
            }
        }
        installed.push(Installed {
            id: id.to_owned(),
            version,
            dir,
            parts: set.artifacts.len(),
            megabytes: set.total_size() / 1_000_000,
        });
    }
    installed
}

fn throw_away_home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

#[tokio::test]
#[ignore = "needs internet access and downloads several hundred MB"]
async fn real_archives_have_the_layout_the_adapters_expect() {
    let (_tmp, home) = throw_away_home();
    let started = Instant::now();
    let installed = install_everything(&home).await;
    assert!(!installed.is_empty(), "nothing to check on this platform");

    let os = Os::current();
    let mut problems = Vec::new();
    for runtime in &installed {
        let adapter = adapter_for(&runtime.id).unwrap();
        let binaries = adapter.binaries(&runtime.dir, os);
        let missing: Vec<_> = binaries
            .iter()
            .filter(|b| b.required && !b.path.is_file())
            .map(|b| b.path.display().to_string())
            .collect();
        let optional_present = binaries
            .iter()
            .filter(|b| !b.required && b.path.exists())
            .count();
        let optional_total = binaries.iter().filter(|b| !b.required).count();
        eprintln!(
            "{} {}: {} parts, {} MB, optional files {optional_present}/{optional_total}{}",
            runtime.id,
            runtime.version,
            runtime.parts,
            runtime.megabytes,
            if missing.is_empty() { "" } else { ", MISSING" }
        );
        if !missing.is_empty() {
            problems.push(format!(
                "{} {}: missing {missing:?}",
                runtime.id, runtime.version
            ));
        }
        for dir in adapter.path_dirs(&runtime.dir, os) {
            if !dir.is_dir() {
                problems.push(format!(
                    "{}: PATH folder {} does not exist",
                    runtime.id,
                    dir.display()
                ));
            }
        }
    }
    eprintln!("checked in {:?}", started.elapsed());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[tokio::test]
#[ignore = "needs internet access, downloads several hundred MB and RUNS the downloaded programs"]
async fn real_binaries_report_the_installed_version() {
    let (_tmp, home) = throw_away_home();
    let installed = install_everything(&home).await;
    assert!(!installed.is_empty(), "nothing to check on this platform");

    let os = Os::current();
    let mut failures = Vec::new();
    for runtime in &installed {
        let adapter = adapter_for(&runtime.id).unwrap();
        match adapter.verify(&runtime.dir, os, &runtime.version) {
            Ok(info) => eprintln!(
                "{} {}: OK: {}",
                runtime.id,
                runtime.version,
                info.output.lines().next().unwrap_or("").trim()
            ),
            Err(err) => {
                eprintln!(
                    "{} {}: FAILED: {}",
                    runtime.id, runtime.version, err.message
                );
                failures.push(format!(
                    "{} {}: {}",
                    runtime.id, runtime.version, err.message
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
