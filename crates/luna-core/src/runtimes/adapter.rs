//! The `RuntimeAdapter` trait: what lunaRun needs to know about one kind of runtime to install, find
//! and check it (docs/ARCHITECTURE.md "Runtime adapter trait"). Adding a runtime is a registry entry
//! plus an adapter.
//!
//! Adapters describe layouts as pure functions of `(install folder, OS)`, so every platform's layout is
//! testable on any machine. Only [`RuntimeAdapter::verify`] touches the disk and starts a process, and it
//! starts nothing but a program inside the install folder, with an argument list, never a shell string.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::diagnostics::{Arch, Os};
use crate::error::{ErrorCode, LunaError, Result};
use crate::paths::LunaHome;

/// How long `--version` and friends may take before the runtime counts as broken.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

/// What an adapter gets for [`RuntimeAdapter::post_install`].
#[derive(Debug, Clone, Copy)]
pub struct InstallCtx<'a> {
    /// The staging folder the archives were extracted into. Files that stay in the install belong here.
    pub dir: &'a Path,
    /// Where the install will be once it is complete. The staging folder is renamed to it, so anything that
    /// must contain an absolute path (a php.ini's `extension_dir`) uses this one, not `dir`.
    pub final_dir: &'a Path,
    /// For files outside the install folder, such as `config/php/<version>/php.ini`.
    pub home: &'a LunaHome,
    pub os: Os,
    pub arch: Arch,
    pub version: &'a str,
}

/// A program a runtime provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
    /// Stable name other code asks for: `php`, `php-fpm`, `mysqld`.
    pub name: &'static str,
    pub path: PathBuf,
    /// A runtime without it is broken. Optional ones (helpers such as `npx`) are reported if present.
    pub required: bool,
}

/// A command that proves an install works, as data: program plus arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionCommand {
    pub program: PathBuf,
    pub args: Vec<&'static str>,
}

/// The result of a successful [`RuntimeAdapter::verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    /// The version the runtime reports, such as `8.3.36`.
    pub version: String,
    /// What it printed, for the diagnostics report.
    pub output: String,
}

pub trait RuntimeAdapter: Send + Sync {
    /// The registry id: `php`, `node`, ...
    fn id(&self) -> &'static str;

    /// Every program the runtime provides, under `dir` (the folder with the receipt, archives already
    /// extracted with their `stripComponents` applied).
    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary>;

    /// Folders that belong on `PATH` for a terminal with this runtime selected.
    fn path_dirs(&self, dir: &Path, os: Os) -> Vec<PathBuf>;

    /// The command whose output proves the install works, or `None` when a runtime cannot be run on its
    /// own (Composer needs a PHP) and [`verify`](Self::verify) is overridden.
    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand>;

    /// The version inside that command's output.
    fn parse_version(&self, output: &str) -> Option<String>;

    /// Work after extraction that is specific to the runtime (a `php.ini`, `codesign` on macOS).
    /// Nothing yet: M1.3 task 3.
    fn post_install(&self, ctx: &InstallCtx<'_>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Check that the install in `dir` is complete and is `expected` (the registry version).
    ///
    /// Default: every required program exists (and is executable on Unix), the version command runs
    /// within [`VERIFY_TIMEOUT`], and its reported version equals `expected`.
    fn verify(&self, dir: &Path, os: Os, expected: &str) -> Result<VersionInfo> {
        check_binaries(self.id(), &self.binaries(dir, os))?;
        let Some(command) = self.version_command(dir, os) else {
            return Err(LunaError::internal(format!(
                "{} has no version command and no custom verify",
                self.id()
            )));
        };
        let output = run_version_command(self.id(), dir, &command, VERIFY_TIMEOUT)?;
        let Some(version) = self.parse_version(&output) else {
            return Err(broken(
                self.id(),
                format!(
                    "could not read a version from its output: {:?}",
                    first_line(&output)
                ),
            ));
        };
        // A pre-release is registered as `8.5.0-rc1` but its binary prints the numbers only.
        let wanted = expected.split('-').next().unwrap_or(expected);
        if version != wanted {
            return Err(broken(
                self.id(),
                format!("it reports version {version} but {expected} was installed"),
            ));
        }
        Ok(VersionInfo { version, output })
    }
}

fn broken(runtime: &str, message: String) -> LunaError {
    LunaError::new(
        ErrorCode::RuntimeBroken,
        format!("the installed {runtime} does not work: {message}"),
    )
    .with_details(json!({ "runtime": runtime }))
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

/// The file name of a program on `os`: `php` becomes `php.exe` on Windows.
pub(crate) fn exe(os: Os, name: &str) -> String {
    if os == Os::Windows {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// The leading `1.2.3` of `text` (digits, dot, digits, dot, digits), ignoring what follows: `11.8.9-MariaDB`
/// gives `11.8.9`.
pub(crate) fn leading_version(text: &str) -> Option<String> {
    let mut parts = 0;
    let mut end = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            break;
        }
        parts += 1;
        end = i;
        if parts == 3 || i >= bytes.len() || bytes[i] != b'.' {
            break;
        }
        i += 1; // the dot
    }
    (parts == 3).then(|| text[..end].to_owned())
}

/// The text right after the first occurrence of `marker`.
pub(crate) fn after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    text.find(marker).map(|at| &text[at + marker.len()..])
}

/// Every required binary exists as a file, and on Unix has an execute bit.
pub(crate) fn check_binaries(runtime: &str, binaries: &[Binary]) -> Result<()> {
    for binary in binaries.iter().filter(|b| b.required) {
        let shown = binary.path.display();
        let meta = std::fs::metadata(&binary.path)
            .map_err(|_| broken(runtime, format!("{shown} is missing")))?;
        if !meta.is_file() {
            return Err(broken(runtime, format!("{shown} is not a file")));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 == 0 {
                return Err(broken(runtime, format!("{shown} is not executable")));
            }
        }
    }
    Ok(())
}

/// Run `command` inside `dir` and return stdout and stderr together.
///
/// Rule 6 (CLAUDE.md): the program must live inside the install folder, arguments are a list, and no shell
/// is involved. A program that takes longer than `timeout` is killed and counts as broken.
pub fn run_version_command(
    runtime: &str,
    dir: &Path,
    command: &VersionCommand,
    timeout: Duration,
) -> Result<String> {
    if !command.program.starts_with(dir) {
        return Err(LunaError::internal(format!(
            "{} would run a program outside its install folder: {}",
            runtime,
            command.program.display()
        )));
    }
    run_program(runtime, &command.program, &command.args, dir, timeout)
}

/// How often a start is retried when the file is momentarily "busy", and how long to wait in between.
const BUSY_RETRIES: u32 = 8;
const BUSY_WAIT: Duration = Duration::from_millis(50);

/// Run `start`, trying again a few times when it fails with "text file busy" (`ETXTBSY`, errno 26 on
/// Linux and macOS). A program that was written a moment ago can still be open for writing in a child
/// that another thread forked in that instant, and `exec` refuses it until that child has exec'd too.
/// It passes in milliseconds, so waiting is the standard answer. Does nothing on Windows.
fn retry_if_busy<T>(mut start: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut attempt = 0;
    loop {
        match start() {
            Err(err) if is_busy(&err) && attempt < BUSY_RETRIES => {
                attempt += 1;
                thread::sleep(BUSY_WAIT);
            }
            other => return other,
        }
    }
}

fn is_busy(err: &std::io::Error) -> bool {
    cfg!(unix) && err.raw_os_error() == Some(26)
}

/// Start `program` with `args` in `cwd` and return stdout and stderr together. Kills it after `timeout`.
pub(crate) fn run_program<S: AsRef<std::ffi::OsStr>>(
    runtime: &str,
    program: &Path,
    args: &[S],
    cwd: &Path,
    timeout: Duration,
) -> Result<String> {
    let mut process = Command::new(program);
    process
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up in the app
    }
    let mut child = retry_if_busy(|| process.spawn()).map_err(|err| {
        broken(
            runtime,
            format!("{} could not be started: {err}", program.display()),
        )
    })?;

    // Read both pipes on their own threads so a chatty program can never fill a pipe and hang.
    let (sender, receiver) = mpsc::channel::<String>();
    let mut readers = 0;
    if let Some(mut out) = child.stdout.take() {
        let sender = sender.clone();
        readers += 1;
        thread::spawn(move || {
            let mut text = String::new();
            let _ = out.read_to_string(&mut text);
            let _ = sender.send(text);
        });
    }
    if let Some(mut err) = child.stderr.take() {
        let sender = sender.clone();
        readers += 1;
        thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            let _ = sender.send(text);
        });
    }
    drop(sender);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(err) => {
                return Err(broken(runtime, format!("waiting for it failed: {err}")));
            }
        }
    };
    // Killing a program does not kill what it started, and a grandchild can keep a pipe open long after
    // the program is gone (a `.cmd` wrapper around a long job). So collecting the output is bounded too:
    // what has arrived by then is all there is, and the reader threads end on their own.
    let mut output = String::new();
    let patience = if status.is_some() {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(200)
    };
    for _ in 0..readers {
        if let Ok(text) = receiver.recv_timeout(patience) {
            output.push_str(&text);
        }
    }
    match status {
        None => Err(broken(
            runtime,
            format!("{} did not finish within {timeout:?}", program.display()),
        )),
        Some(status) if !status.success() => Err(broken(
            runtime,
            format!(
                "{} exited with {status}: {:?}",
                program.display(),
                first_line(&output)
            ),
        )),
        Some(_) => Ok(output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_version_reads_three_numbers_and_ignores_the_rest() {
        assert_eq!(leading_version("8.3.36 (cli)").as_deref(), Some("8.3.36"));
        assert_eq!(
            leading_version("11.8.9-MariaDB for Linux").as_deref(),
            Some("11.8.9")
        );
        assert_eq!(leading_version("24.22.2").as_deref(), Some("24.22.2"));
        assert_eq!(leading_version("10.11.19").as_deref(), Some("10.11.19"));
        for bad in ["", "8.3", "8", "v8.3.1", "a.b.c", "8..3", ".8.3.1", "8.3.x"] {
            assert_eq!(leading_version(bad), None, "{bad:?}");
        }
        // A fourth number is not part of a version line.
        assert_eq!(leading_version("1.2.3.4").as_deref(), Some("1.2.3"));
    }

    #[cfg(unix)]
    #[test]
    fn a_busy_file_is_retried_a_few_times_and_other_errors_are_not() {
        use std::io::{Error, ErrorKind};

        let mut calls = 0;
        let result = retry_if_busy(|| {
            calls += 1;
            if calls < 4 {
                Err(Error::from_raw_os_error(26))
            } else {
                Ok("started")
            }
        });
        assert_eq!(result.unwrap(), "started");
        assert_eq!(calls, 4);

        // Never busy forever: after the last retry the error comes back.
        let mut calls = 0;
        let err = retry_if_busy::<()>(|| {
            calls += 1;
            Err(Error::from_raw_os_error(26))
        })
        .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(26));
        assert_eq!(calls, BUSY_RETRIES + 1);

        // Any other failure is final at once.
        let mut calls = 0;
        let err = retry_if_busy::<()>(|| {
            calls += 1;
            Err(Error::from(ErrorKind::NotFound))
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(calls, 1);
    }

    #[test]
    fn after_returns_the_text_following_a_marker() {
        assert_eq!(after("mysqld  Ver 8.4.13 for", " Ver "), Some("8.4.13 for"));
        assert_eq!(after("nothing here", " Ver "), None);
    }

    #[test]
    fn programs_outside_the_install_folder_are_refused_before_anything_starts() {
        let command = VersionCommand {
            program: PathBuf::from("/usr/bin/somewhere-else"),
            args: vec!["-v"],
        };
        let err = run_version_command(
            "php",
            Path::new("/data/lunarun/php"),
            &command,
            VERIFY_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
    }
}
