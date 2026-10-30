//! The job manager running a real install-style pipeline: download, verify and extract as phases of one
//! job, against a local HTTP server. It proves cancellation works in every phase, failures keep their
//! typed error, failed jobs clean up after themselves, and duplicates are deduplicated.
//! Every test runs in a data folder whose name has a space and non-ASCII characters (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod archive_builders;
mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use archive_builders::{Entry, incompressible, zip};
use luna_core::download::{DownloadOptions, DownloadRequest, Downloader};
use luna_core::extract::{ArchiveFormat, ExtractOptions, extract};
use luna_core::jobs::{JobContext, JobEvent, JobManager, JobOptions, JobState};
use luna_core::{ErrorCode, LunaHome, Result};
use sha2::{Digest, Sha256};
use support::{Server, send_part_of, serve};
use tempfile::TempDir;
use tokio::runtime::Handle;

const ARCHIVE_NAME: &str = "php-8.3.14.zip";

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Rig {
    _tmp: TempDir,
    home: LunaHome,
    jobs: JobManager,
    events: Arc<Mutex<Vec<JobEvent>>>,
    staging: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        home.ensure_layout().unwrap();
        let events: Arc<Mutex<Vec<JobEvent>>> = Arc::default();
        let sink = Arc::clone(&events);
        let jobs = JobManager::new(
            Handle::current(),
            move |event| sink.lock().unwrap().push(event),
            JobOptions {
                progress_interval: Duration::ZERO,
                max_finished: 50,
            },
        );
        let staging = home.runtimes_dir().join("php").join(".staging-test");
        Self {
            _tmp: tmp,
            home,
            jobs,
            events,
            staging,
        }
    }

    /// Start the install pipeline for `url`. The key is `php@8.3.14`.
    fn start_install(&self, url: String, sha256: String) -> luna_core::jobs::StartedJob {
        let home = self.home.clone();
        let staging = self.staging.clone();
        self.jobs
            .start("php@8.3.14", "Install PHP 8.3.14", move |ctx| {
                install(ctx, home, url, sha256, staging)
            })
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

    fn done_events(&self) -> Vec<luna_core::jobs::JobDone> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| {
                if let JobEvent::Done(d) = e {
                    Some(d.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// What the runtime manager will do in M1.3: download, verify (inside the download), extract into a
/// staging folder, and clean up after itself when anything goes wrong.
async fn install(
    ctx: JobContext,
    home: LunaHome,
    url: String,
    sha256: String,
    staging: PathBuf,
) -> Result<()> {
    let downloader = Downloader::new(DownloadOptions::loopback_test())?;

    ctx.phase("download", None);
    let request = DownloadRequest {
        url: &url,
        file_name: ARCHIVE_NAME,
        sha256: &sha256,
    };
    let progress = ctx.clone();
    let archive = downloader
        .download(&home, &request, ctx.cancel_token(), move |p| {
            progress.progress("download", p.bytes_done, p.bytes_total, None);
        })
        .await?;

    ctx.phase("extract", None);
    let progress = ctx.clone();
    let result = extract(
        archive.clone(),
        staging.clone(),
        ArchiveFormat::Zip,
        ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        },
        ctx.cancel_token().clone(),
        move |p| progress.progress("extract", p.bytes_written, p.bytes_total, None),
    )
    .await;

    if let Err(error) = result {
        home.safe_remove(&staging)?; // never leave a half-extracted runtime behind
        return Err(error);
    }
    home.safe_remove(&archive)?;
    Ok(())
}

fn small_runtime() -> Vec<u8> {
    zip(&[
        Entry::dir("php/"),
        Entry::file("php/php.exe", b"binary"),
        Entry::file("php/php.ini", b"ini"),
    ])
}

#[tokio::test]
async fn a_full_install_runs_through_all_phases_and_ends_with_one_done_event() {
    let rig = Rig::new();
    let archive = small_runtime();
    let served = archive.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));

    let started = rig.start_install(server.url("/php.zip"), sha256_hex(&archive));
    let info = rig.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Succeeded, "{:?}", info.error);
    assert_eq!(fs::read(rig.staging.join("php.exe")).unwrap(), b"binary");
    assert!(
        !rig.home.downloads_dir().join(ARCHIVE_NAME).exists(),
        "the downloaded archive is removed"
    );

    assert_eq!(rig.phases(), vec!["starting", "download", "extract"]);
    let done = rig.done_events();
    assert_eq!(done.len(), 1);
    assert!(done[0].ok && done[0].error.is_none());
    assert!(
        matches!(rig.events.lock().unwrap().last(), Some(JobEvent::Done(_))),
        "Done is the final event"
    );
}

#[tokio::test]
async fn cancelling_during_the_download_keeps_the_partial_file_and_extracts_nothing() {
    let rig = Rig::new();
    let archive = small_runtime();
    let half = archive.len() / 2;
    let served = archive.clone();
    // Sends half of the body, then goes quiet.
    let server = Server::start(move |_, _, stream| {
        send_part_of(stream, &served, 0, half, Some(Duration::from_secs(5)))
    });

    let started = rig.start_install(server.url("/php.zip"), sha256_hex(&archive));
    for _ in 0..500 {
        let info = rig.jobs.get(&started.id).unwrap();
        if info.phase == "download" && info.bytes_done > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(rig.jobs.cancel(&started.id).unwrap());

    let info = rig.jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Cancelled);
    assert_eq!(info.error.unwrap().code, ErrorCode::Cancelled);
    assert!(
        rig.home
            .downloads_dir()
            .join(format!("{ARCHIVE_NAME}.part"))
            .exists(),
        "kept for a later resume"
    );
    assert!(!rig.staging.exists(), "extraction never started");
    let done = rig.done_events();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].error.as_ref().unwrap().code, ErrorCode::Cancelled);
}

#[tokio::test]
async fn cancelling_during_extraction_removes_the_staging_folder() {
    let rig = Rig::new();
    let entries: Vec<Entry> = (0..30)
        .map(|i| Entry::file(&format!("php/f{i}.bin"), incompressible(200_000)))
        .collect();
    let archive = zip(&entries);
    let served = archive.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));

    let started = rig.start_install(server.url("/php.zip"), sha256_hex(&archive));
    for _ in 0..2000 {
        let info = rig.jobs.get(&started.id).unwrap();
        if info.phase == "extract" && info.bytes_done > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    rig.jobs.cancel(&started.id).unwrap();

    let info = rig.jobs.wait(&started.id).await.unwrap();
    assert_eq!(info.state, JobState::Cancelled, "{:?}", info.error);
    assert_eq!(info.error.unwrap().code, ErrorCode::Cancelled);
    assert!(
        !rig.staging.exists(),
        "a half-extracted runtime must not be left behind"
    );
}

#[tokio::test]
async fn a_checksum_mismatch_fails_the_job_with_that_code_and_leaves_nothing() {
    let rig = Rig::new();
    let archive = small_runtime();
    let server = Server::start(move |req, _, stream| serve(stream, req, &archive));

    let started = rig.start_install(server.url("/php.zip"), sha256_hex(b"not the archive"));
    let info = rig.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Failed);
    assert_eq!(info.error.unwrap().code, ErrorCode::ChecksumMismatch);
    assert!(!rig.staging.exists());
    assert!(!rig.home.downloads_dir().join(ARCHIVE_NAME).exists());
    assert!(
        !rig.home
            .downloads_dir()
            .join(format!("{ARCHIVE_NAME}.part"))
            .exists()
    );
    assert_eq!(
        rig.done_events()[0].error.as_ref().unwrap().code,
        ErrorCode::ChecksumMismatch
    );
}

#[tokio::test]
async fn a_hostile_archive_fails_the_job_and_nothing_escapes_the_staging_folder() {
    let rig = Rig::new();
    let archive = zip(&[
        Entry::file("php/ok.txt", b"fine"),
        Entry::file("../../evil.txt", b"pwned"),
    ]);
    let served = archive.clone();
    let server = Server::start(move |req, _, stream| serve(stream, req, &served));

    let started = rig.start_install(server.url("/php.zip"), sha256_hex(&archive));
    let info = rig.jobs.wait(&started.id).await.unwrap();

    assert_eq!(info.state, JobState::Failed);
    assert_eq!(info.error.unwrap().code, ErrorCode::ArchiveUnsafe);
    assert!(
        !rig.staging.exists(),
        "the partially extracted staging folder was removed"
    );
    assert!(!rig.home.root().join("evil.txt").exists());
    assert!(!rig.home.runtimes_dir().join("evil.txt").exists());
}

#[tokio::test]
async fn starting_the_same_install_twice_runs_it_once() {
    let rig = Rig::new();
    let archive = small_runtime();
    let half = archive.len() / 2;
    let served = archive.clone();
    // The first request stalls, so the first job is still running when the second start arrives.
    let server = Server::start(move |req, nth, stream| {
        if nth == 0 {
            send_part_of(stream, &served, 0, half, Some(Duration::from_millis(400)));
        } else {
            serve(stream, req, &served);
        }
    });
    let url = server.url("/php.zip");
    let sha = sha256_hex(&archive);

    let first = rig.start_install(url.clone(), sha.clone());
    let second = rig.start_install(url, sha);
    assert!(!first.already_running);
    assert!(second.already_running);
    assert_eq!(second.id, first.id);

    rig.jobs.wait(&first.id).await.unwrap();
    assert_eq!(rig.done_events().len(), 1, "one job, one Done event");
    let jobs = rig.jobs.list();
    assert_eq!(jobs.iter().filter(|j| j.key == "php@8.3.14").count(), 1);
}
