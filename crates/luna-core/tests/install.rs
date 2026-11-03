//! The install flow against a fake network and real archives built in code. `verify` really runs a small
//! script, so the whole flow (download, extract, merge, post-install, verify, receipt, rename) is exercised.
//! Every data folder has a space and non-ASCII characters in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod archive_builders;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use archive_builders::{Entry, tar_gz, tar_xz, zip};
use luna_core::diagnostics::{Arch, Os};
use luna_core::download::{DownloadOptions, Downloader, Progress};
use luna_core::error::ErrorCode;
use luna_core::jobs::{JobEvent, JobManager, JobOptions, JobState};
use luna_core::registry::Registry;
use luna_core::runtimes::{
    Binary, FetchRequest, Fetcher, InstallCtx, InstalledRuntime, Installer, RECEIPT_FILE,
    RuntimeAdapter, VersionCommand, VersionInfo, cleanup_incomplete, list_installed,
};
use luna_core::{LunaError, LunaHome};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::runtime::Handle;

// ---------------------------------------------------------------------------------------------
// A runtime that is really a script
// ---------------------------------------------------------------------------------------------

fn os() -> Os {
    Os::current()
}

fn script_name() -> &'static str {
    if cfg!(windows) { "tool.cmd" } else { "tool.sh" }
}

/// The content of a script that prints `tool <version>`.
fn script(version: &str) -> Vec<u8> {
    if cfg!(windows) {
        format!("@echo off\r\necho tool {version}\r\n").into_bytes()
    } else {
        format!("#!/bin/sh\necho tool {version}\n").into_bytes()
    }
}

fn tool(path: &str, version: &str) -> Entry {
    Entry::file(path, script(version)).mode(0o755)
}

/// Every `post_install` call the test adapter saw: the staging folder and the final folder.
static POST_INSTALL: Mutex<Vec<(PathBuf, PathBuf, PathBuf)>> = Mutex::new(Vec::new());

/// An adapter for the test runtimes. `scripted` runs `tool.cmd`/`tool.sh`. `payload` has no program and
/// only checks that its file arrived. Both refuse to verify when a receipt already exists, which proves
/// the receipt is written last.
struct Scripted;

impl RuntimeAdapter for Scripted {
    fn id(&self) -> &'static str {
        "scripted"
    }
    fn binaries(&self, dir: &Path, _os: Os) -> Vec<Binary> {
        vec![Binary {
            name: "tool",
            path: dir.join(script_name()),
            required: true,
        }]
    }
    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.to_path_buf()]
    }
    fn version_command(&self, dir: &Path, _os: Os) -> Option<VersionCommand> {
        Some(VersionCommand {
            program: dir.join(script_name()),
            args: vec!["--version"],
        })
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output.split_whitespace().nth(1).map(str::to_owned)
    }
    fn post_install(&self, ctx: &InstallCtx<'_>) -> Result<(), LunaError> {
        POST_INSTALL.lock().unwrap().push((
            ctx.dir.to_path_buf(),
            ctx.final_dir.to_path_buf(),
            ctx.home.root().to_path_buf(),
        ));
        Ok(())
    }
    fn verify(&self, dir: &Path, os: Os, expected: &str) -> Result<VersionInfo, LunaError> {
        assert!(
            !dir.join(RECEIPT_FILE).exists(),
            "the receipt must be written after verify"
        );
        assert!(
            !dir.to_string_lossy().ends_with(expected),
            "verify runs in the staging folder, not the final one"
        );
        // The default behaviour, spelled out because a trait default cannot be called from an override.
        let binaries = self.binaries(dir, os);
        for binary in &binaries {
            if !binary.path.is_file() {
                return Err(LunaError::new(
                    ErrorCode::RuntimeBroken,
                    format!("{} is missing", binary.path.display()),
                ));
            }
        }
        let command = self.version_command(dir, os).unwrap();
        let output = luna_core::runtimes::run_version_command(
            "scripted",
            dir,
            &command,
            Duration::from_secs(20),
        )?;
        let version = self
            .parse_version(&output)
            .ok_or_else(|| LunaError::new(ErrorCode::RuntimeBroken, "no version in the output"))?;
        if version != expected {
            return Err(LunaError::new(
                ErrorCode::RuntimeBroken,
                format!("it reports version {version} but {expected} was installed"),
            ));
        }
        Ok(VersionInfo { version, output })
    }
}

/// A runtime that is just a file (like Composer's phar), checked by existing.
struct Payload;

impl RuntimeAdapter for Payload {
    fn id(&self) -> &'static str {
        "payload"
    }
    fn binaries(&self, dir: &Path, _os: Os) -> Vec<Binary> {
        vec![Binary {
            name: "payload",
            path: dir.join("payload.bin"),
            required: true,
        }]
    }
    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.to_path_buf()]
    }
    fn version_command(&self, _dir: &Path, _os: Os) -> Option<VersionCommand> {
        None
    }
    fn parse_version(&self, _output: &str) -> Option<String> {
        None
    }
    fn verify(&self, dir: &Path, _os: Os, expected: &str) -> Result<VersionInfo, LunaError> {
        assert!(!dir.join(RECEIPT_FILE).exists());
        if !dir.join("payload.bin").is_file() {
            return Err(LunaError::new(
                ErrorCode::RuntimeBroken,
                "payload.bin is missing",
            ));
        }
        Ok(VersionInfo {
            version: expected.to_owned(),
            output: String::new(),
        })
    }
}

fn adapters(id: &str) -> Option<&'static dyn RuntimeAdapter> {
    match id {
        "scripted" | "twopart" => Some(&Scripted),
        "payload" => Some(&Payload),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// A fake network
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Net {
    files: Mutex<HashMap<String, Vec<u8>>>,
    fetched: Mutex<Vec<String>>,
    failing: Mutex<HashMap<String, ErrorCode>>,
    /// Leave a `.part` file and wait for the job to be cancelled.
    block: AtomicBool,
    /// Cancel the job right after the download finished.
    cancel_after_download: AtomicBool,
    calls: AtomicUsize,
}

impl Net {
    fn serve(&self, url: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(url.to_owned(), bytes);
    }
    fn fetched(&self) -> Vec<String> {
        self.fetched.lock().unwrap().clone()
    }
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn fetcher(net: &Arc<Net>, home: &LunaHome) -> Fetcher {
    let net = Arc::clone(net);
    let home = home.clone();
    Arc::new(move |mut request: FetchRequest| {
        let net = Arc::clone(&net);
        let home = home.clone();
        Box::pin(async move {
            net.calls.fetch_add(1, Ordering::SeqCst);
            net.fetched.lock().unwrap().push(request.url.clone());
            let downloads = home.downloads_dir();
            fs::create_dir_all(&downloads).unwrap();
            if let Some(code) = net.failing.lock().unwrap().get(&request.url) {
                return Err(LunaError::new(*code, "the fake network failed"));
            }
            if net.block.load(Ordering::SeqCst) {
                fs::write(
                    downloads.join(format!("{}.part", request.file_name)),
                    "partial",
                )
                .unwrap();
                request.cancel.cancelled().await;
                return Err(LunaError::new(ErrorCode::Cancelled, "cancelled"));
            }
            let bytes = net
                .files
                .lock()
                .unwrap()
                .get(&request.url)
                .cloned()
                .unwrap();
            let len = bytes.len() as u64;
            (request.on_progress)(Progress {
                bytes_done: len / 2,
                bytes_total: Some(len),
            });
            (request.on_progress)(Progress {
                bytes_done: len,
                bytes_total: Some(len),
            });
            if sha256_hex(&bytes) != request.sha256 {
                return Err(LunaError::new(
                    ErrorCode::ChecksumMismatch,
                    "wrong checksum",
                ));
            }
            let path = downloads.join(&request.file_name);
            fs::write(&path, bytes).unwrap();
            if net.cancel_after_download.load(Ordering::SeqCst) {
                request.cancel.cancel();
            }
            Ok(path)
        })
    })
}

// ---------------------------------------------------------------------------------------------
// The test bench
// ---------------------------------------------------------------------------------------------

struct Part {
    url: &'static str,
    format: &'static str,
    variant: Option<&'static str>,
    strip: usize,
    requires: Vec<&'static str>,
    bytes: Vec<u8>,
}

impl Part {
    fn new(url: &'static str, format: &'static str, bytes: Vec<u8>) -> Self {
        Self {
            url,
            format,
            variant: None,
            strip: 0,
            requires: Vec::new(),
            bytes,
        }
    }
}

struct Bench {
    _tmp: TempDir,
    home: LunaHome,
    net: Arc<Net>,
    jobs: JobManager,
    events: Arc<Mutex<Vec<JobEvent>>>,
}

fn os_name() -> &'static str {
    match os() {
        Os::Windows => "windows",
        Os::Macos => "macos",
        _ => "linux",
    }
}

fn arch_name() -> &'static str {
    if Arch::current() == Arch::Aarch64 {
        "aarch64"
    } else {
        "x86_64"
    }
}

impl Bench {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        let events: Arc<Mutex<Vec<JobEvent>>> = Arc::default();
        let sink = Arc::clone(&events);
        let jobs = JobManager::new(
            Handle::current(),
            move |event| sink.lock().unwrap().push(event),
            JobOptions {
                progress_interval: Duration::ZERO,
                ..JobOptions::default()
            },
        );
        Self {
            _tmp: tmp,
            home,
            net: Arc::default(),
            jobs,
            events,
        }
    }

    fn installer(&self) -> Installer {
        let downloader = Downloader::new(DownloadOptions::loopback_test()).unwrap();
        Installer::new(self.home.clone(), downloader)
            .with_fetcher(fetcher(&self.net, &self.home))
            .with_adapters(adapters)
            .with_free_space(Arc::new(|_| Ok(u64::MAX)))
    }

    /// Publish `parts` as `runtime` `version` for this computer and serve them from the fake network.
    fn registry(&self, runtime: &str, version: &str, parts: &[Part]) -> Arc<Registry> {
        let artifacts: Vec<Value> = parts
            .iter()
            .map(|part| {
                self.net.serve(part.url, part.bytes.clone());
                let mut value = json!({
                    "os": os_name(), "arch": arch_name(), "format": part.format,
                    "url": part.url, "sha256": sha256_hex(&part.bytes),
                    "size": part.bytes.len(), "stripComponents": part.strip,
                    "requires": part.requires,
                });
                if let Some(variant) = part.variant {
                    value["variant"] = json!(variant);
                }
                value
            })
            .collect();
        let text = json!({ "schema": 1, "runtimes": { runtime: {
            "displayName": "Scripted", "versions": [ { "version": version, "artifacts": artifacts } ]
        } } })
        .to_string();
        Arc::new(Registry::parse(&text).unwrap())
    }

    /// Run an install as a job and return its result.
    async fn install(
        &self,
        installer: &Installer,
        registry: &Arc<Registry>,
        runtime: &str,
        version: &str,
    ) -> Result<InstalledRuntime, LunaError> {
        let slot: Arc<Mutex<Option<Result<InstalledRuntime, LunaError>>>> = Arc::default();
        let (installer, registry) = (installer.clone(), Arc::clone(registry));
        let (runtime_id, version_text) = (runtime.to_owned(), version.to_owned());
        let out = Arc::clone(&slot);
        let started = self.jobs.start(
            &format!(
                "direct-{runtime}@{version}-{}",
                self.events.lock().unwrap().len()
            ),
            "test",
            move |ctx| async move {
                let result = installer
                    .install(&registry, &runtime_id, &version_text, &ctx)
                    .await;
                let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
                *out.lock().unwrap() = Some(result);
                outcome
            },
        );
        self.jobs.wait(&started.id).await.unwrap();
        slot.lock().unwrap().take().unwrap()
    }

    fn runtime_dir(&self, runtime: &str) -> PathBuf {
        self.home.runtimes_dir().join(runtime)
    }

    /// Names inside `runtimes/<runtime>`.
    fn names(&self, runtime: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.runtime_dir(runtime))
            .map(|dir| {
                dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn downloads(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.home.downloads_dir())
            .map(|dir| {
                dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn phases(&self) -> Vec<String> {
        let mut phases: Vec<String> = Vec::new();
        for event in self.events.lock().unwrap().iter() {
            if let JobEvent::Progress(p) = event
                && phases.last() != Some(&p.phase)
            {
                phases.push(p.phase.clone());
            }
        }
        phases
    }
}

fn only_installed(bench: &Bench) -> Vec<String> {
    list_installed(&bench.home)
        .unwrap()
        .iter()
        .map(|i| format!("{} {}", i.runtime, i.version))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Success paths
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn every_archive_format_installs_with_its_wrapper_folder_stripped() {
    for (format, bytes) in [
        (
            "zip",
            zip(&[
                tool("wrapper/tool.cmd", "1.0.0"),
                tool("wrapper/tool.sh", "1.0.0"),
            ]),
        ),
        (
            "tar.gz",
            tar_gz(&[
                tool("wrapper/tool.cmd", "1.0.0"),
                tool("wrapper/tool.sh", "1.0.0"),
            ]),
        ),
        (
            "tar.xz",
            tar_xz(&[
                tool("wrapper/tool.cmd", "1.0.0"),
                tool("wrapper/tool.sh", "1.0.0"),
            ]),
        ),
    ] {
        let bench = Bench::new();
        let mut part = Part::new("https://example.test/scripted.pkg", format, bytes);
        part.strip = 1;
        let registry = bench.registry("scripted", "1.0.0", &[part]);

        let installed = bench
            .install(&bench.installer(), &registry, "scripted", "1.0.0")
            .await
            .unwrap_or_else(|e| panic!("{format}: {e}"));

        assert_eq!(
            installed.dir,
            bench.home.runtime_dir("scripted", "1.0.0").unwrap()
        );
        assert!(installed.dir.join(script_name()).is_file(), "{format}");
        assert!(
            !installed.dir.join("wrapper").exists(),
            "{format}: wrapper stripped"
        );
        assert_eq!(only_installed(&bench), ["scripted 1.0.0"], "{format}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn post_install_is_told_the_staging_folder_the_final_folder_and_the_home() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    let installed = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap();

    let seen = POST_INSTALL.lock().unwrap().clone();
    let (staging, final_dir, home_root) = seen
        .iter()
        .find(|(_, _, home)| home == bench.home.root())
        .expect("post_install ran for this home");
    assert_eq!(
        final_dir, &installed.dir,
        "the folder the install ends up in"
    );
    assert_ne!(staging, final_dir, "the work happens in a staging folder");
    assert!(
        staging
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".staging-")
    );
    assert_eq!(home_root, bench.home.root());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_successful_install_leaves_exactly_the_install_and_nothing_else() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );

    let installed = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap();

    assert_eq!(
        bench.names("scripted"),
        ["1.0.0"],
        "no staging or extract folders"
    );
    assert!(
        bench.downloads().is_empty(),
        "the archive is deleted: {:?}",
        bench.downloads()
    );
    let receipt = &installed.receipt;
    assert_eq!(
        (receipt.runtime.as_str(), receipt.version.as_str()),
        ("scripted", "1.0.0")
    );
    assert_eq!(receipt.os, os_name());
    assert_eq!(receipt.arch, arch_name());
    assert!(!receipt.rosetta);
    assert_eq!(receipt.artifacts.len(), 1);
    assert_eq!(receipt.artifacts[0].url, "https://example.test/a.zip");
    assert!(installed.dir.join(RECEIPT_FILE).is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_job_reports_each_phase_in_order_with_progress_over_all_parts() {
    let bench = Bench::new();
    let first = zip(&[tool(script_name(), "1.0.0")]);
    let second = zip(&[Entry::file("second.txt", "two")]);
    let (first_len, second_len) = (first.len() as u64, second.len() as u64);
    let mut a = Part::new("https://example.test/cli.zip", "zip", first);
    a.variant = Some("static-cli");
    let mut b = Part::new("https://example.test/fpm.zip", "zip", second);
    b.variant = Some("static-fpm");
    let registry = bench.registry("twopart", "1.0.0", &[a, b]);

    bench
        .install(&bench.installer(), &registry, "twopart", "1.0.0")
        .await
        .unwrap();

    assert_eq!(
        bench.phases(),
        [
            "starting",
            "preparing",
            "downloading",
            "extracting",
            "verifying",
            "finishing"
        ]
    );
    let downloaded: Vec<u64> = bench
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            JobEvent::Progress(p) if p.phase == "downloading" => {
                assert_eq!(
                    p.bytes_total,
                    Some(first_len + second_len),
                    "the total covers every part"
                );
                Some(p.bytes_done)
            }
            _ => None,
        })
        .collect();
    assert!(
        downloaded.windows(2).all(|w| w[0] <= w[1]),
        "never goes backwards: {downloaded:?}"
    );
    assert_eq!(*downloaded.last().unwrap(), first_len + second_len);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_flat_archives_are_merged_into_one_install_folder() {
    let bench = Bench::new();
    let mut cli = Part::new(
        "https://example.test/cli.tar.gz",
        "tar.gz",
        tar_gz(&[tool(script_name(), "1.0.0")]),
    );
    cli.variant = Some("static-cli");
    let mut fpm = Part::new(
        "https://example.test/fpm.tar.gz",
        "tar.gz",
        tar_gz(&[
            Entry::file("second.txt", "two"),
            Entry::file("lib/helper.txt", "h"),
        ]),
    );
    fpm.variant = Some("static-fpm");
    let registry = bench.registry("twopart", "1.0.0", &[cli, fpm]);

    let installed = bench
        .install(&bench.installer(), &registry, "twopart", "1.0.0")
        .await
        .unwrap();

    assert!(installed.dir.join(script_name()).is_file());
    assert_eq!(
        fs::read_to_string(installed.dir.join("second.txt")).unwrap(),
        "two"
    );
    assert!(installed.dir.join("lib").join("helper.txt").is_file());
    assert_eq!(installed.receipt.artifacts.len(), 2);
    assert_eq!(
        installed.receipt.artifacts[0].variant.as_deref(),
        Some("static-cli")
    );
    assert_eq!(bench.names("twopart"), ["1.0.0"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_downloads_with_the_same_file_name_do_not_overwrite_each_other() {
    let bench = Bench::new();
    // Both addresses end in `pkg.zip`, as real upstream files can. They must be told apart on disk.
    let mut cli = Part::new(
        "https://example.test/cli/pkg.zip",
        "zip",
        zip(&[tool(script_name(), "1.0.0")]),
    );
    cli.variant = Some("static-cli");
    let mut fpm = Part::new(
        "https://example.test/fpm/pkg.zip",
        "zip",
        zip(&[Entry::file("second.txt", "from the second download")]),
    );
    fpm.variant = Some("static-fpm");
    let registry = bench.registry("twopart", "1.0.0", &[cli, fpm]);

    let installed = bench
        .install(&bench.installer(), &registry, "twopart", "1.0.0")
        .await
        .unwrap();

    assert!(installed.dir.join(script_name()).is_file());
    assert_eq!(
        fs::read_to_string(installed.dir.join("second.txt")).unwrap(),
        "from the second download"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn parts_that_both_contain_the_same_file_are_a_registry_error_and_install_nothing() {
    let bench = Bench::new();
    let a = Part::new(
        "https://example.test/a.zip",
        "zip",
        zip(&[tool(script_name(), "1.0.0")]),
    );
    let mut b = Part::new(
        "https://example.test/b.zip",
        "zip",
        zip(&[tool(script_name(), "1.0.0")]),
    );
    b.variant = Some("other");
    let mut a = a;
    a.variant = Some("one");
    let registry = bench.registry("twopart", "1.0.0", &[a, b]);

    let err = bench
        .install(&bench.installer(), &registry, "twopart", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::RegistryInvalid);
    assert!(only_installed(&bench).is_empty());
    assert_eq!(
        bench.names("twopart"),
        Vec::<String>::new(),
        "work folders removed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_file_download_is_placed_under_the_name_in_its_address() {
    let bench = Bench::new();
    let part = Part::new(
        "https://example.test/dl/payload.bin",
        "file",
        b"phar bytes".to_vec(),
    );
    let registry = bench.registry("payload", "2.0.0", &[part]);

    let installed = bench
        .install(&bench.installer(), &registry, "payload", "2.0.0")
        .await
        .unwrap();

    assert_eq!(
        fs::read(installed.dir.join("payload.bin")).unwrap(),
        b"phar bytes"
    );
    assert!(bench.downloads().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_intel_build_installed_on_apple_silicon_is_recorded_as_a_rosetta_install() {
    let bench = Bench::new();
    // The registry only has an Intel macOS build, and this "computer" is an Apple silicon Mac.
    let part = Part::new(
        "https://example.test/a.zip",
        "zip",
        zip(&[tool(script_name(), "1.0.0")]),
    );
    let text = json!({ "schema": 1, "runtimes": { "scripted": {
        "displayName": "Scripted", "versions": [ { "version": "1.0.0", "artifacts": [ {
            "os": "macos", "arch": "x86_64", "format": "zip", "url": part.url,
            "sha256": sha256_hex(&part.bytes), "size": part.bytes.len()
        } ] } ]
    } } })
    .to_string();
    bench.net.serve(part.url, part.bytes.clone());
    let registry = Arc::new(Registry::parse(&text).unwrap());
    let installer = bench.installer().with_platform(Os::Macos, Arch::Aarch64);

    let installed = bench
        .install(&installer, &registry, "scripted", "1.0.0")
        .await
        .unwrap();

    assert!(installed.receipt.rosetta);
    assert_eq!(installed.receipt.os, "macos");
    assert_eq!(
        installed.receipt.arch, "aarch64",
        "the computer, not the build"
    );
}

// ---------------------------------------------------------------------------------------------
// Refusals before any download
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_prerequisite_stops_the_install_before_anything_is_downloaded() {
    let bench = Bench::new();
    let mut part = Part::new(
        "https://example.test/a.zip",
        "zip",
        zip(&[tool(script_name(), "1.0.0")]),
    );
    part.requires = vec!["vcredist-14-x64", "other", "vcredist-14-x64"];
    let registry = bench.registry("scripted", "1.0.0", &[part]);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let record = Arc::clone(&seen);
    let installer = bench
        .installer()
        .with_prerequisites(Arc::new(move |requires| {
            *record.lock().unwrap() = requires.to_vec();
            Err(LunaError::new(
                ErrorCode::PrerequisiteMissing,
                "vcredist-14-x64 is not installed",
            ))
        }));

    let err = bench
        .install(&installer, &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert_eq!(
        *seen.lock().unwrap(),
        ["other", "vcredist-14-x64"],
        "sorted and without duplicates"
    );
    assert!(bench.net.fetched().is_empty(), "nothing was downloaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_windows_install_without_the_visual_cpp_runtime_is_refused_before_any_download() {
    use luna_core::runtimes::prerequisites::{
        VCREDIST_DOWNLOAD_URL, VCREDIST_ID, VcRedist, check_requirements,
    };

    let bench = Bench::new();
    let mut part = Part::new(
        "https://example.test/a.zip",
        "zip",
        zip(&[tool(script_name(), "1.0.0")]),
    );
    part.requires = vec![VCREDIST_ID];
    let registry = bench.registry("scripted", "1.0.0", &[part]);

    // The registry says Windows needs it, and the PC does not have it.
    let missing = bench.installer().with_prerequisites(Arc::new(|requires| {
        check_requirements(Os::Windows, requires, &|| Ok(None))
    }));
    let err = bench
        .install(&missing, &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert_eq!(err.details.unwrap()["downloadUrl"], VCREDIST_DOWNLOAD_URL);
    assert!(bench.net.fetched().is_empty(), "nothing was downloaded");
    assert!(only_installed(&bench).is_empty());
    assert!(bench.names("scripted").is_empty());

    // Once it is installed, the same install goes through.
    let present = bench.installer().with_prerequisites(Arc::new(|requires| {
        check_requirements(Os::Windows, requires, &|| {
            Ok(Some(VcRedist {
                installed: true,
                major: 14,
                minor: 51,
                build: 36247,
                version: None,
            }))
        })
    }));
    bench
        .install(&present, &registry, "scripted", "1.0.0")
        .await
        .unwrap();
    assert_eq!(only_installed(&bench), ["scripted 1.0.0"]);
}

/// A stand-in for `reg.exe`: prints `output` and exits with `code`.
fn fake_reg(dir: &Path, output: &str, code: i32) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    if cfg!(windows) {
        let path = dir.join("reg.cmd");
        let echoes: String = output
            .lines()
            .map(|l| {
                format!(
                    "echo {l}
"
                )
            })
            .collect();
        fs::write(
            &path,
            format!(
                "@echo off
{echoes}exit /b {code}
"
            ),
        )
        .unwrap();
        path
    } else {
        let path = dir.join("reg.sh");
        fs::write(
            &path,
            format!(
                "#!/bin/sh
cat <<'EOF'
{output}
EOF
exit {code}
"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installers_own_check_reads_the_registry_through_reg_when_nothing_replaces_it() {
    let bench = Bench::new();
    // A Windows artifact that needs the Visual C++ runtime, installed on a "Windows PC", whatever the host.
    let bytes = zip(&[tool(script_name(), "1.0.0")]);
    bench.net.serve("https://example.test/a.zip", bytes.clone());
    let text = json!({ "schema": 1, "runtimes": { "scripted": {
        "displayName": "Scripted", "versions": [ { "version": "1.0.0", "artifacts": [ {
            "os": "windows", "arch": "x86_64", "format": "zip", "url": "https://example.test/a.zip",
            "sha256": sha256_hex(&bytes), "size": bytes.len(), "requires": ["vcredist-14-x64"]
        } ] } ]
    } } })
    .to_string();
    let registry = Arc::new(Registry::parse(&text).unwrap());
    let windows = bench.installer().with_platform(Os::Windows, Arch::X86_64);
    let tools = bench._tmp.path().join("fake tools");

    // `reg` says the key does not exist: refused, before any download.
    let absent =
        windows
            .clone()
            .with_reg_path(fake_reg(&tools.join("absent"), "ERROR: key not found", 1));
    let err = bench
        .install(&absent, &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert!(bench.net.fetched().is_empty(), "nothing was downloaded");

    // `reg` reports a recent runtime: the install goes through.
    let output = "    Installed    REG_DWORD    0x1
    Major    REG_DWORD    0xe
    Minor    REG_DWORD    0x33
    Bld    REG_DWORD    0x8d97";
    let present = windows.with_reg_path(fake_reg(&tools.join("present"), output, 0));
    bench
        .install(&present, &registry, "scripted", "1.0.0")
        .await
        .unwrap();
    assert_eq!(only_installed(&bench), ["scripted 1.0.0"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn too_little_disk_space_stops_the_install_before_anything_is_downloaded() {
    let bench = Bench::new();
    let bytes = zip(&[tool(script_name(), "1.0.0")]);
    let size = bytes.len() as u64;
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new("https://example.test/a.zip", "zip", bytes)],
    );
    let needed = size * 3;

    let short = bench
        .installer()
        .with_free_space(Arc::new(move |_| Ok(needed - 1)));
    let err = bench
        .install(&short, &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InsufficientDisk);
    let details = err.details.unwrap();
    assert_eq!(details["needed"], needed);
    assert_eq!(details["free"], needed - 1);
    assert!(bench.net.fetched().is_empty());

    // Exactly enough is enough.
    let enough = bench
        .installer()
        .with_free_space(Arc::new(move |_| Ok(needed)));
    bench
        .install(&enough, &registry, "scripted", "1.0.0")
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_free_space_does_not_block_the_install() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    let installer = bench
        .installer()
        .with_free_space(Arc::new(|_| Err(std::io::Error::other("no such volume"))));
    bench
        .install(&installer, &registry, "scripted", "1.0.0")
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn installing_the_same_version_twice_is_refused_before_any_download() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    let installer = bench.installer();
    bench
        .install(&installer, &registry, "scripted", "1.0.0")
        .await
        .unwrap();
    let before = bench.net.fetched().len();
    let receipt_before = fs::read(
        bench
            .home
            .runtime_dir("scripted", "1.0.0")
            .unwrap()
            .join(RECEIPT_FILE),
    )
    .unwrap();

    let err = bench
        .install(&installer, &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::AlreadyInstalled);
    assert_eq!(bench.net.fetched().len(), before, "no second download");
    assert_eq!(
        fs::read(
            bench
                .home
                .runtime_dir("scripted", "1.0.0")
                .unwrap()
                .join(RECEIPT_FILE)
        )
        .unwrap(),
        receipt_before,
        "the existing install is untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_runtimes_versions_platforms_and_adapters_are_typed_errors() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    let installer = bench.installer();

    let no_version = bench
        .install(&installer, &registry, "scripted", "9.9.9")
        .await
        .unwrap_err();
    assert_eq!(no_version.code, ErrorCode::NotFound);

    let no_adapter = bench
        .install(&installer, &registry, "ruby", "1.0.0")
        .await
        .unwrap_err();
    assert_eq!(no_adapter.code, ErrorCode::NotFound);
    assert!(no_adapter.message.contains("no adapter"));

    // A computer the registry has no build for.
    let other = if os() == Os::Windows {
        Os::Linux
    } else {
        Os::Windows
    };
    let elsewhere = installer.with_platform(other, Arch::X86_64);
    let err = bench
        .install(&elsewhere, &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NoArtifactForPlatform);

    assert!(bench.net.fetched().is_empty());
    assert!(only_installed(&bench).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Failures after work has started leave nothing behind
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_checksum_installs_nothing() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    // The server now answers with other bytes than the registry promised.
    bench.net.serve(
        "https://example.test/a.zip",
        zip(&[tool(script_name(), "6.6.6")]),
    );

    let err = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(only_installed(&bench).is_empty());
    assert!(bench.names("scripted").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hostile_archive_is_refused_and_leaves_nothing_inside_or_outside_the_home() {
    let bench = Bench::new();
    let evil = zip(&[
        tool(script_name(), "1.0.0"),
        Entry::file("../../escaped.txt", "pwned"),
    ]);
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new("https://example.test/evil.zip", "zip", evil)],
    );

    let err = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::ArchiveUnsafe);
    assert!(only_installed(&bench).is_empty());
    assert!(
        bench.names("scripted").is_empty(),
        "staging and extract folders removed"
    );
    assert!(bench.downloads().is_empty(), "the bad archive is not kept");
    let escaped = bench.home.root().parent().unwrap().join("escaped.txt");
    assert!(!escaped.exists());
    assert!(!bench.home.root().join("escaped.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_that_does_not_report_the_installed_version_is_not_installed() {
    let bench = Bench::new();
    // The registry says 1.0.0, the program says 2.0.0.
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "2.0.0")]),
        )],
    );

    let err = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::RuntimeBroken);
    assert!(only_installed(&bench).is_empty());
    assert!(
        bench.names("scripted").is_empty(),
        "the failed install is removed"
    );
    assert!(bench.downloads().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_program_in_the_archive_is_not_installed() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[Entry::file("README.txt", "hi")]),
        )],
    );
    let err = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RuntimeBroken);
    assert!(bench.names("scripted").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_install_does_not_disturb_an_installed_version() {
    let bench = Bench::new();
    let good = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    let installer = bench.installer();
    bench
        .install(&installer, &good, "scripted", "1.0.0")
        .await
        .unwrap();

    let broken = bench.registry(
        "scripted",
        "1.1.0",
        &[Part::new(
            "https://example.test/b.zip",
            "zip",
            zip(&[tool(script_name(), "9.9.9")]),
        )],
    );
    bench
        .install(&installer, &broken, "scripted", "1.1.0")
        .await
        .unwrap_err();

    assert_eq!(only_installed(&bench), ["scripted 1.0.0"]);
    assert_eq!(bench.names("scripted"), ["1.0.0"]);
}

// ---------------------------------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_the_download_keeps_the_partial_file_and_installs_nothing() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    bench.net.block.store(true, Ordering::SeqCst);
    let started = bench
        .installer()
        .start(&bench.jobs, Arc::clone(&registry), "scripted", "1.0.0")
        .unwrap();

    // Wait until the fetch is really running, then cancel through the job manager.
    while bench.net.calls.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(bench.jobs.cancel(&started.id).unwrap());
    let info = bench.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Cancelled);
    assert_eq!(info.error.unwrap().code, ErrorCode::Cancelled);
    assert!(only_installed(&bench).is_empty());
    assert!(bench.names("scripted").is_empty());
    assert!(
        bench.downloads().iter().any(|n| n.ends_with(".part")),
        "the partial download stays so the next try can resume: {:?}",
        bench.downloads()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_after_the_download_keeps_the_archive_and_leaves_no_work_folders() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    bench
        .net
        .cancel_after_download
        .store(true, Ordering::SeqCst);

    let err = bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(only_installed(&bench).is_empty());
    assert!(bench.names("scripted").is_empty());
    assert_eq!(
        bench.downloads().len(),
        1,
        "a finished download is kept for the retry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_request_for_the_same_runtime_and_version_joins_the_running_job() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    bench.net.block.store(true, Ordering::SeqCst);
    let installer = bench.installer();

    let first = installer
        .start(&bench.jobs, Arc::clone(&registry), "scripted", "1.0.0")
        .unwrap();
    let second = installer
        .start(&bench.jobs, Arc::clone(&registry), "scripted", "1.0.0")
        .unwrap();
    assert!(!first.already_running);
    assert!(second.already_running);
    assert_eq!(first.id, second.id);

    // Let the download really start, so "one download" is a fact and not a race.
    while bench.net.calls.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    bench.jobs.cancel(&first.id).unwrap();
    bench.jobs.wait(&first.id).await.unwrap();
    assert_eq!(
        bench.net.calls.load(Ordering::SeqCst),
        1,
        "one download, not two"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_started_job_installs_and_is_labelled_with_the_runtime_name() {
    let bench = Bench::new();
    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    let started = bench
        .installer()
        .start(&bench.jobs, registry, "scripted", "1.0.0")
        .unwrap();
    let info = bench.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Succeeded);
    assert_eq!(info.key, "scripted@1.0.0");
    assert_eq!(info.label, "Install Scripted 1.0.0");
    assert_eq!(only_installed(&bench), ["scripted 1.0.0"]);
}

// ---------------------------------------------------------------------------------------------
// Startup cleanup
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cleanup_removes_only_the_work_folders_an_install_left_behind() {
    let bench = Bench::new();
    assert!(
        cleanup_incomplete(&bench.home).unwrap().is_empty(),
        "no runtimes folder yet"
    );

    let registry = bench.registry(
        "scripted",
        "1.0.0",
        &[Part::new(
            "https://example.test/a.zip",
            "zip",
            zip(&[tool(script_name(), "1.0.0")]),
        )],
    );
    bench
        .install(&bench.installer(), &registry, "scripted", "1.0.0")
        .await
        .unwrap();

    let runtime = bench.runtime_dir("scripted");
    fs::create_dir_all(runtime.join(".staging-job-7").join("deep")).unwrap();
    fs::write(runtime.join(".staging-job-7").join("deep").join("x"), "x").unwrap();
    fs::create_dir_all(runtime.join(".extract-job-7-1")).unwrap();
    fs::create_dir_all(runtime.join(".keep-me")).unwrap(); // not ours
    fs::create_dir_all(bench.home.runtimes_dir().join("php").join(".staging-job-3")).unwrap();
    fs::create_dir_all(bench.home.downloads_dir()).unwrap();
    fs::write(
        bench.home.downloads_dir().join("abc-php.zip.part"),
        "partial",
    )
    .unwrap();

    let removed = cleanup_incomplete(&bench.home).unwrap();

    assert_eq!(removed.len(), 3);
    assert_eq!(bench.names("scripted"), [".keep-me", "1.0.0"]);
    assert_eq!(bench.names("php"), Vec::<String>::new());
    assert_eq!(
        only_installed(&bench),
        ["scripted 1.0.0"],
        "the real install is untouched"
    );
    assert!(
        bench.home.downloads_dir().join("abc-php.zip.part").exists(),
        "partial downloads are kept for resuming"
    );
}
