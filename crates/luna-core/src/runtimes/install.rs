//! Installing a runtime (FR-RT-02, FR-RT-03, FR-RT-13, docs/RUNTIMES.md section 4).
//!
//! ```text
//! resolve parts -> prerequisites -> free disk -> download each part (SHA-256 verified)
//!   -> extract each archive into its own folder and merge into runtimes/<rt>/.staging-<job>/
//!   -> post_install -> verify (runs the new binary) -> receipt -> atomic rename to runtimes/<rt>/<version>/
//! ```
//!
//! Everything happens inside hidden `.staging-*` / `.extract-*` folders, so a folder with a plain version
//! name is only ever a complete, verified install, and a failure, a cancel or a crash leaves nothing a
//! later listing could mistake for an install. [`cleanup_incomplete`] removes what a crash left behind.

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use reqwest::Url;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::adapter::{InstallCtx, RuntimeAdapter};
use super::adapters::adapter_for;
use super::installed::{InstallReceipt, InstalledRuntime};
use super::prerequisites::{default_check, default_reg_path};
use crate::diagnostics::{Arch, Os};
use crate::download::{DownloadRequest, Downloader, Progress};
use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::extract::{ExtractOptions, ExtractProgress, extract};
use crate::jobs::{JobContext, JobManager, StartedJob};
use crate::paths::{LunaHome, component};
use crate::registry::{Artifact, ArtifactFormat, Registry};

/// Free space wanted, as a multiple of the download size: the archive, its extracted files and room
/// to spare.
pub const DISK_FACTOR: u64 = 3;

/// One download for the [`Fetcher`].
pub struct FetchRequest {
    pub url: String,
    /// A single path component, unique per file: it names the file inside `LUNA_HOME/downloads`.
    pub file_name: String,
    pub sha256: String,
    pub cancel: CancellationToken,
    pub on_progress: Box<dyn FnMut(Progress) + Send>,
}

pub type FetchFuture = Pin<Box<dyn Future<Output = Result<PathBuf>> + Send>>;

/// Downloads a file and returns the path of the finished, verified file. The real one wraps
/// [`Downloader`]; tests replace it so no network is needed.
pub type Fetcher = Arc<dyn Fn(FetchRequest) -> FetchFuture + Send + Sync>;

/// The real [`Fetcher`]: resumable, retried, HTTPS-only and SHA-256 verified downloads into
/// `LUNA_HOME/downloads` (`luna-core::download`).
pub fn downloader_fetcher(home: LunaHome, downloader: Downloader) -> Fetcher {
    let downloader = Arc::new(downloader);
    Arc::new(move |request: FetchRequest| {
        let downloader = Arc::clone(&downloader);
        let home = home.clone();
        Box::pin(async move {
            downloader
                .download(
                    &home,
                    &DownloadRequest {
                        url: &request.url,
                        file_name: &request.file_name,
                        sha256: &request.sha256,
                    },
                    &request.cancel,
                    request.on_progress,
                )
                .await
        })
    })
}

/// Bytes free on the disk that holds a path.
pub type FreeSpace = Arc<dyn Fn(&Path) -> std::io::Result<u64> + Send + Sync>;

/// Checks the `requires` of the artifacts (`vcredist-14-x64`). Returns `PREREQUISITE_MISSING` when one is
/// not there. Without this the installer uses the real check (`prerequisites::default_check`).
pub type Prerequisites = Arc<dyn Fn(&[String]) -> Result<()> + Send + Sync>;

pub type AdapterLookup = fn(&str) -> Option<&'static dyn RuntimeAdapter>;

struct Config {
    home: LunaHome,
    fetcher: Fetcher,
    os: Os,
    arch: Arch,
    free_space: FreeSpace,
    /// `None`: the built-in check (`prerequisites::default_check`, the real Windows registry).
    prerequisites: Option<Prerequisites>,
    /// The `reg.exe` the built-in check asks (replaceable in tests).
    reg: PathBuf,
    adapters: AdapterLookup,
}

/// Installs runtimes into a `LUNA_HOME`. Cheap to clone.
#[derive(Clone)]
pub struct Installer(Arc<Config>);

impl Installer {
    /// An installer for this computer that downloads with `downloader`.
    pub fn new(home: LunaHome, downloader: Downloader) -> Self {
        let fetcher = downloader_fetcher(home.clone(), downloader);
        Self(Arc::new(Config {
            home,
            fetcher,
            os: Os::current(),
            arch: Arch::current(),
            free_space: Arc::new(|path| fs4::available_space(path)),
            prerequisites: None,
            reg: default_reg_path(),
            adapters: adapter_for,
        }))
    }

    fn modified(self, change: impl FnOnce(&mut Config)) -> Self {
        let old = &*self.0;
        let mut config = Config {
            home: old.home.clone(),
            fetcher: Arc::clone(&old.fetcher),
            os: old.os,
            arch: old.arch,
            free_space: Arc::clone(&old.free_space),
            prerequisites: old.prerequisites.clone(),
            reg: old.reg.clone(),
            adapters: old.adapters,
        };
        change(&mut config);
        Self(Arc::new(config))
    }

    /// Install for another platform than this computer's (tests).
    #[must_use]
    pub fn with_platform(self, os: Os, arch: Arch) -> Self {
        self.modified(|c| {
            c.os = os;
            c.arch = arch;
        })
    }

    #[must_use]
    pub fn with_fetcher(self, fetcher: Fetcher) -> Self {
        self.modified(|c| c.fetcher = fetcher)
    }

    #[must_use]
    pub fn with_free_space(self, free_space: FreeSpace) -> Self {
        self.modified(|c| c.free_space = free_space)
    }

    #[must_use]
    pub fn with_prerequisites(self, prerequisites: Prerequisites) -> Self {
        self.modified(|c| c.prerequisites = Some(prerequisites))
    }

    /// Where the built-in Visual C++ check finds `reg.exe` (tests point it at a stand-in).
    #[must_use]
    pub fn with_reg_path(self, reg: PathBuf) -> Self {
        self.modified(|c| c.reg = reg)
    }

    #[must_use]
    pub fn with_adapters(self, adapters: AdapterLookup) -> Self {
        self.modified(|c| c.adapters = adapters)
    }

    pub fn home(&self) -> &LunaHome {
        &self.0.home
    }

    /// Start the install as a background job keyed `<runtime>@<version>`. A second request for the same
    /// runtime and version while one runs returns the running job.
    pub fn start(
        &self,
        jobs: &JobManager,
        registry: Arc<Registry>,
        runtime: &str,
        version: &str,
    ) -> StartedJob {
        let installer = self.clone();
        let (runtime_id, version_text) = (runtime.to_owned(), version.to_owned());
        let name = registry
            .runtime(runtime)
            .map_or(runtime, |r| r.display_name.as_str());
        let label = format!("Install {name} {version}");
        jobs.start(
            &format!("{runtime}@{version}"),
            &label,
            move |ctx| async move {
                installer
                    .install(&registry, &runtime_id, &version_text, &ctx)
                    .await
                    .map(|_| ())
            },
        )
    }

    /// Install `runtime` `version` and return it. See the module docs for the steps. Progress goes to `ctx`,
    /// and cancelling `ctx` stops it at the next opportunity and cleans up.
    pub async fn install(
        &self,
        registry: &Registry,
        runtime: &str,
        version: &str,
        ctx: &JobContext,
    ) -> Result<InstalledRuntime> {
        let config = &*self.0;
        let mut work = Work::new(&config.home, ctx.id());
        let result = run(config, registry, runtime, version, ctx, &mut work).await;
        // A cancelled or interrupted download keeps its partial file so the next try resumes. Anything
        // else leaves nothing behind that is worth keeping.
        let keep_downloads = matches!(&result, Err(e) if matches!(e.code, ErrorCode::Cancelled | ErrorCode::Network));
        work.finish(keep_downloads);
        result
    }
}

/// Everything an install creates that has to disappear again, removed when [`Work::finish`] runs or, on a
/// panic, when it is dropped.
struct Work<'a> {
    home: &'a LunaHome,
    id: String,
    folders: Vec<PathBuf>,
    downloads: Vec<PathBuf>,
    done: bool,
}

impl<'a> Work<'a> {
    fn new(home: &'a LunaHome, id: String) -> Self {
        Self {
            home,
            id,
            folders: Vec::new(),
            downloads: Vec::new(),
            done: false,
        }
    }

    fn finish(&mut self, keep_downloads: bool) {
        self.done = true;
        for folder in self.folders.drain(..) {
            if folder.exists() {
                let _ = self.home.safe_remove(&folder);
            }
        }
        if !keep_downloads {
            for file in self.downloads.drain(..) {
                if file.exists() {
                    let _ = self.home.safe_remove(&file);
                }
            }
        }
    }
}

impl Drop for Work<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.finish(true);
        }
    }
}

fn cancelled() -> LunaError {
    LunaError::new(ErrorCode::Cancelled, "the installation was cancelled")
}

fn check_cancelled(ctx: &JobContext) -> Result<()> {
    if ctx.cancel_token().is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn invalid(message: String) -> LunaError {
    LunaError::new(ErrorCode::RegistryInvalid, message)
}

async fn run(
    config: &Config,
    registry: &Registry,
    runtime: &str,
    version: &str,
    ctx: &JobContext,
    work: &mut Work<'_>,
) -> Result<InstalledRuntime> {
    let home = &config.home;
    let (os, arch) = (config.os, config.arch);

    ctx.phase("preparing", None);
    let adapter = (config.adapters)(runtime).ok_or_else(|| {
        LunaError::new(
            ErrorCode::NotFound,
            format!("lunaRun cannot install {runtime}: it has no adapter for it"),
        )
    })?;
    let final_dir = home.runtime_dir(runtime, version)?;
    if final_dir.exists() {
        return Err(already_installed(runtime, version));
    }
    let set = registry.artifacts_for(runtime, version, os, arch)?;
    check_cancelled(ctx)?;

    let mut requires: Vec<String> = set
        .artifacts
        .iter()
        .flat_map(|a| a.requires.iter().cloned())
        .collect();
    requires.sort();
    requires.dedup();
    match &config.prerequisites {
        Some(check) => check(&requires)?,
        // Reads the registry by starting `reg.exe`, so it must not run on an async worker thread.
        None => {
            let (wanted, reg) = (requires.clone(), config.reg.clone());
            tokio::task::spawn_blocking(move || default_check(os, &wanted, &reg))
                .await
                .map_err(|err| {
                    LunaError::internal(format!("the prerequisite check failed: {err}"))
                })??;
        }
    }

    home.ensure_layout()?;
    let needed = set.total_size().saturating_mul(DISK_FACTOR);
    // If the free space cannot be read, do not block the install: it will fail on its own if the disk is full.
    if let Ok(free) = (config.free_space)(home.root())
        && free < needed
    {
        return Err(LunaError::new(
            ErrorCode::InsufficientDisk,
            format!(
                "installing {runtime} {version} needs about {} MB of free disk space and only {} MB are free",
                needed / 1_000_000,
                free / 1_000_000
            ),
        )
        .with_details(json!({ "needed": needed, "free": free })));
    }

    // 1. Download every part.
    let total = set.total_size();
    let mut archives: Vec<(&Artifact, PathBuf)> = Vec::new();
    let mut done_before = 0u64;
    for artifact in &set.artifacts {
        check_cancelled(ctx)?;
        let base = url_file_name(&artifact.url)?;
        let progress_ctx = ctx.clone();
        let offset = done_before;
        let shown = base.clone();
        let path = (config.fetcher)(FetchRequest {
            url: artifact.url.clone(),
            file_name: format!("{}-{base}", &artifact.sha256[..12]),
            sha256: artifact.sha256.clone(),
            cancel: ctx.cancel_token().clone(),
            on_progress: Box::new(move |progress| {
                progress_ctx.progress(
                    "downloading",
                    offset + progress.bytes_done,
                    Some(total),
                    Some(&shown),
                );
            }),
        })
        .await?;
        work.downloads.push(path.clone());
        archives.push((artifact, path));
        done_before += artifact.size;
    }

    // 2. Extract each archive into its own folder, then merge them into the staging folder.
    let runtime_dir = home.runtimes_dir().join(component(runtime)?);
    let staging = runtime_dir.join(format!(".staging-{}", work.id));
    if staging.exists() {
        home.safe_remove(&staging)?; // a leftover with the same name from an earlier run
    }
    fs::create_dir_all(&staging).at_path(&staging)?;
    work.folders.push(staging.clone());

    ctx.phase("extracting", None);
    for (index, (artifact, archive)) in archives.iter().enumerate() {
        check_cancelled(ctx)?;
        match artifact.format {
            ArtifactFormat::File => {
                let name = url_file_name(&artifact.url)?;
                let target = staging.join(&name);
                if target.exists() {
                    return Err(parts_collide(runtime, version, &name));
                }
                fs::copy(archive, &target).at_path(&target)?;
            }
            format => {
                let archive_format = format
                    .archive_format()
                    .ok_or_else(|| LunaError::internal("an archive format without an extractor"))?;
                let part = runtime_dir.join(format!(".extract-{}-{index}", work.id));
                work.folders.push(part.clone());
                let options = ExtractOptions {
                    strip_components: artifact.strip_components,
                    ..ExtractOptions::default()
                };
                let progress_ctx = ctx.clone();
                extract(
                    archive.clone(),
                    part.clone(),
                    archive_format,
                    options,
                    ctx.cancel_token().clone(),
                    move |p: ExtractProgress| {
                        progress_ctx.progress("extracting", p.bytes_written, p.bytes_total, None);
                    },
                )
                .await?;
                merge_top_level(&part, &staging, runtime, version)?;
            }
        }
    }

    // 3. Make it ready and prove it works. This starts the freshly installed program.
    check_cancelled(ctx)?;
    ctx.phase("verifying", None);
    adapter.post_install(&InstallCtx {
        dir: &staging,
        final_dir: &final_dir,
        home,
        os,
        arch,
        version,
    })?;
    let (verify_dir, expected) = (staging.clone(), version.to_owned());
    tokio::task::spawn_blocking(move || adapter.verify(&verify_dir, os, &expected))
        .await
        .map_err(|err| LunaError::internal(format!("the verification task failed: {err}")))??;

    // 4. Receipt last, then the atomic rename that makes it an installed runtime.
    check_cancelled(ctx)?;
    ctx.phase("finishing", None);
    let receipt = InstallReceipt::new(
        runtime,
        version,
        os,
        arch,
        set.rosetta_fallback,
        &set.artifacts,
    );
    receipt.write(&staging)?;
    if final_dir.exists() {
        // Someone else installed it while we were working.
        return Err(already_installed(runtime, version));
    }
    fs::rename(&staging, &final_dir).at_path(&final_dir)?;
    work.folders.retain(|folder| folder != &staging);

    Ok(InstalledRuntime {
        runtime: runtime.to_owned(),
        version: version.to_owned(),
        dir: final_dir,
        receipt,
    })
}

fn already_installed(runtime: &str, version: &str) -> LunaError {
    LunaError::new(
        ErrorCode::AlreadyInstalled,
        format!("{runtime} {version} is already installed"),
    )
    .with_details(json!({ "runtime": runtime, "version": version }))
}

fn parts_collide(runtime: &str, version: &str, name: &str) -> LunaError {
    invalid(format!(
        "the downloads that make up {runtime} {version} both contain {name:?}"
    ))
}

/// The file name at the end of a download address, which must be a safe single path component.
fn url_file_name(url: &str) -> Result<String> {
    let parsed = Url::parse(url).map_err(|_| invalid(format!("{url:?} is not a valid URL")))?;
    let name = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or_default();
    component(name)
        .map(str::to_owned)
        .map_err(|_| invalid(format!("{url:?} does not end in a usable file name")))
}

/// Move every entry of `from` into `to` without overwriting anything. How the parts of one install (PHP's
/// `php` and `php-fpm` archives) end up in one folder.
fn merge_top_level(from: &Path, to: &Path, runtime: &str, version: &str) -> Result<()> {
    for entry in fs::read_dir(from).at_path(from)? {
        let entry = entry.at_path(from)?;
        let target = to.join(entry.file_name());
        if fs::symlink_metadata(&target).is_ok() {
            return Err(parts_collide(
                runtime,
                version,
                &entry.file_name().to_string_lossy(),
            ));
        }
        fs::rename(entry.path(), &target).at_path(&target)?;
    }
    Ok(())
}

/// Remove what an install that died halfway left behind: every `.staging-*` and `.extract-*` folder under
/// `runtimes/<runtime>/`. Run it at startup, before any install. Partial downloads are kept on purpose
/// (they let the next try resume). Returns the folders it removed.
pub fn cleanup_incomplete(home: &LunaHome) -> Result<Vec<PathBuf>> {
    let root = home.runtimes_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for runtime in fs::read_dir(&root).at_path(&root)? {
        let runtime = runtime.at_path(&root)?;
        if !runtime.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(children) = fs::read_dir(runtime.path()) else {
            continue;
        };
        for child in children.flatten() {
            let name = child.file_name().to_string_lossy().into_owned();
            let is_work_folder = name.starts_with(".staging-") || name.starts_with(".extract-");
            if is_work_folder && child.file_type().is_ok_and(|t| t.is_dir()) {
                home.safe_remove(&child.path())?;
                removed.push(child.path());
            }
        }
    }
    removed.sort();
    Ok(removed)
}
