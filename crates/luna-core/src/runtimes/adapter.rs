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

    /// Folders outside the install folder that belong to this version and go with it when it is uninstalled
    /// (PHP's `config/php/<version>`). Nothing by default.
    fn config_dirs(&self, home: &LunaHome, version: &str) -> Vec<PathBuf> {
        let _ = (home, version);
        Vec::new()
    }

    /// Folders that hold the user's **data** for this version, a database's data directory. Uninstalling
    /// keeps them unless the user asks to delete them (FR-RT-06), and keeps them while another installed
    /// version still names the same folder (patch versions of one line share it). Nothing by default.
    fn data_dirs(&self, home: &LunaHome, version: &str) -> Vec<PathBuf> {
        let _ = (home, version);
        Vec::new()
    }

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

/// How often a folder move is retried when Windows refuses it, and how long to wait in between (10 s in all).
const RENAME_RETRIES: u32 = 40;
const RENAME_WAIT: Duration = Duration::from_millis(250);

/// Windows refuses to move a folder while any file in it is open without the "may be moved" permission, and the
/// virus scanner and the search indexer open every freshly written program that way for a moment: a MySQL that
/// was just unpacked and run for its version check could not be moved into place ("Access is denied", error 5;
/// 32 is the sharing violation). It passes within seconds, so the move is tried again for a while. Other errors,
/// and every error on macOS and Linux, are returned at once.
fn is_held_by_another_program(err: &std::io::Error) -> bool {
    cfg!(windows) && matches!(err.raw_os_error(), Some(5 | 32))
}

fn retry_while_held<T>(
    mut op: impl FnMut() -> std::io::Result<T>,
    retries: u32,
    wait: Duration,
) -> std::io::Result<T> {
    let mut attempt = 0;
    loop {
        match op() {
            Err(err) if is_held_by_another_program(&err) && attempt < retries => {
                attempt += 1;
                thread::sleep(wait);
            }
            other => return other,
        }
    }
}

/// `std::fs::rename` for a runtime folder that a scanner may still be looking at (see
/// [`is_held_by_another_program`]).
pub(crate) fn rename_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    retry_while_held(|| std::fs::rename(from, to), RENAME_RETRIES, RENAME_WAIT)
}

/// What a program printed and how it ended, for callers that care about a non-zero exit (`reg query` exits
/// with 1 when a key does not exist, which is an answer, not a failure).
pub(crate) struct Finished {
    pub success: bool,
    /// The exit status as text, for messages.
    pub status: String,
    /// Stdout and stderr together.
    pub output: String,
}

/// Start `program` with `args` in `cwd` and return stdout and stderr together. A program that cannot be
/// started, that is killed after `timeout`, or that exits with a failure is an error.
pub(crate) fn run_program<S: AsRef<std::ffi::OsStr>>(
    runtime: &str,
    program: &Path,
    args: &[S],
    cwd: &Path,
    timeout: Duration,
) -> Result<String> {
    let finished = run_program_raw(runtime, program, args, cwd, timeout)?;
    if finished.success {
        Ok(finished.output)
    } else {
        Err(broken(
            runtime,
            format!(
                "{} exited with {}: {:?}",
                program.display(),
                finished.status,
                first_line(&finished.output)
            ),
        ))
    }
}

/// Everything `reader` yields, as text. Bytes that are not UTF-8 become U+FFFD instead of discarding the lot:
/// `mysqld --version` starts its line with its own path, which Windows programs write in the ANSI code page,
/// so a profile folder like `C:\Users\José` made the whole output unreadable and the install "broken" (NFR-07).
fn read_lossy(reader: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    // What arrived before a read error is still worth having.
    let _ = reader.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Like [`run_program`], but a non-zero exit is returned, not turned into an error. Failing to start the
/// program or running past `timeout` still is one.
pub(crate) fn run_program_raw<S: AsRef<std::ffi::OsStr>>(
    runtime: &str,
    program: &Path,
    args: &[S],
    cwd: &Path,
    timeout: Duration,
) -> Result<Finished> {
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
            let _ = sender.send(read_lossy(&mut out));
        });
    }
    if let Some(mut err) = child.stderr.take() {
        let sender = sender.clone();
        readers += 1;
        thread::spawn(move || {
            let _ = sender.send(read_lossy(&mut err));
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
        Some(status) => Ok(Finished {
            success: status.success(),
            status: status.to_string(),
            output,
        }),
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

    #[test]
    fn output_that_is_not_utf8_keeps_its_readable_part() {
        // "C:\Users\Jos<0xE9>\mysqld.exe  Ver 9.7.4 ...": a Latin-1 byte in the path, the version after it.
        let mut raw: &[u8] = b"C:\\Users\\Jos\xE9\\mysqld.exe  Ver 9.7.4 for Win64\n";
        let text = read_lossy(&mut raw);
        assert!(text.contains(" Ver 9.7.4"), "{text:?}");
        assert_eq!(
            super::super::adapters::Mysql
                .parse_version(&text)
                .as_deref(),
            Some("9.7.4")
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_that_is_held_for_a_moment_is_moved_once_it_is_let_go() {
        use std::io::Error;

        for code in [5, 32] {
            let mut calls = 0;
            let result = retry_while_held(
                || {
                    calls += 1;
                    if calls < 4 {
                        Err(Error::from_raw_os_error(code))
                    } else {
                        Ok("moved")
                    }
                },
                10,
                Duration::from_millis(1),
            );
            assert_eq!(result.unwrap(), "moved", "error {code}");
            assert_eq!(calls, 4);
        }

        // Never forever: the error comes back after the last retry.
        let mut calls = 0;
        let err = retry_while_held::<()>(
            || {
                calls += 1;
                Err(Error::from_raw_os_error(5))
            },
            3,
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(5));
        assert_eq!(calls, 4, "one try and three retries");
    }

    #[test]
    fn other_errors_are_not_retried_when_moving_a_folder() {
        use std::io::{Error, ErrorKind};

        let mut calls = 0;
        let err = retry_while_held::<()>(
            || {
                calls += 1;
                Err(Error::new(ErrorKind::NotFound, "gone"))
            },
            10,
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_folder_is_really_moved_by_rename_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("stag ing");
        let to = tmp.path().join("fertig");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("f.txt"), "x").unwrap();
        rename_dir(&from, &to).unwrap();
        assert!(!from.exists());
        assert!(to.join("f.txt").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_prints_non_utf8_still_has_its_output_returned() {
        let dir = std::env::temp_dir();
        let out = run_program(
            "x",
            Path::new("/bin/sh"),
            &["-c", "printf 'caf\\351 Ver 1.2.3\\n'"],
            &dir,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(out.contains(" Ver 1.2.3"), "{out:?}");
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
