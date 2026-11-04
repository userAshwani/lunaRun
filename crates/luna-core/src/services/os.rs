//! Asking the operating system about a process by its number, and ending one that is not our child.
//!
//! This is what makes "no orphans" possible (FR-SV-07): after a crash of lunaRun the next launch finds a PID file
//! and must know whether that process is still alive and whether it is still the program that was started (the
//! number may have been given to something else since). A few calls per OS, no crate for it (D-024): the
//! `windows-sys` bindings on Windows, `/proc` on Linux, `libproc` on macOS, and POSIX `kill`.

use std::path::{Path, PathBuf};
#[cfg(any(windows, unix))]
use std::time::Duration;

/// Process numbers that are never a service: 0 is not a process (to `kill` it is the caller's whole process group)
/// and 1 is `init` or `launchd`. Asking about them says "no", so a damaged PID file can never point lunaRun at the
/// system.
fn never_a_service(pid: u32) -> bool {
    pid <= 1
}

/// The process with this number exists and is running (a zombie that only waits to be reaped does not count).
pub fn process_alive(pid: u32) -> bool {
    !never_a_service(pid) && imp::alive(pid)
}

/// The program file the process was started from, when the OS tells us. `None`: no such process, or it belongs
/// to somebody else and the OS will not say.
pub fn process_exe(pid: u32) -> Option<PathBuf> {
    if never_a_service(pid) {
        return None;
    }
    imp::exe(pid)
}

/// The same program file? On macOS and Linux that means the same file on disk (device and inode), so a link, a hard
/// link and the original are one program: the OS may report any of the names of a file. Elsewhere, and for a path
/// that does not exist, the paths are compared after resolving links, ignoring case and the `\\?\` prefix on
/// Windows.
pub fn same_program(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    normal(a) == normal(b)
}

fn normal(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.trim_start_matches(r"\\?\").to_lowercase()
    } else {
        text
    }
}

/// End one process at once, whoever started it. Does nothing if it is already gone.
pub(super) fn terminate_process(pid: u32) {
    imp::terminate(pid);
}

/// Ask a process to leave by signal (`SIGTERM`, `SIGQUIT`). Windows has no such thing for a background process:
/// it is terminated.
pub(super) fn ask_to_leave(pid: u32, how: Leave) {
    imp::ask_to_leave(pid, how);
}

#[derive(Clone, Copy)]
pub(super) enum Leave {
    Terminate,
    Quit,
}

/// Kill everything left in the process group that `leader` started (`kill(-pgid, SIGKILL)`). Nothing on Windows,
/// where the Job Object does this.
pub(super) fn kill_group(leader: u32) {
    imp::kill_group(leader);
}

/// Wait until the process is gone, for at most `limit`. `true`: it is gone.
#[cfg(any(windows, unix))]
pub(super) async fn wait_gone(pid: u32, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    while process_alive(pid) {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    true
}

// ---------------------------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::PathBuf;

    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    };

    use super::Leave;

    /// The exit code a running process reports.
    const STILL_ACTIVE: u32 = 259;

    fn open(pid: u32, access: u32) -> Option<OwnedHandle> {
        // SAFETY: plain Win32 call with value arguments; a null result means "no such process or no access".
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            None
        } else {
            // SAFETY: `handle` is a valid handle we own; `OwnedHandle` closes it.
            Some(unsafe { OwnedHandle::from_raw_handle(handle) })
        }
    }

    pub fn alive(pid: u32) -> bool {
        let Some(process) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            return false;
        };
        let mut code = 0_u32;
        // SAFETY: the handle is valid for the call and `code` is a live `u32`.
        let ok = unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) };
        ok != 0 && code == STILL_ACTIVE
    }

    pub fn exe(pid: u32) -> Option<PathBuf> {
        let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut buffer = vec![0_u16; 32_768];
        let mut length = u32::try_from(buffer.len()).ok()?;
        // SAFETY: the buffer holds `length` UTF-16 units and the call writes at most that many, updating `length`.
        let ok = unsafe {
            QueryFullProcessImageNameW(process.as_raw_handle(), 0, buffer.as_mut_ptr(), &mut length)
        };
        if ok == 0 {
            return None;
        }
        buffer.truncate(usize::try_from(length).ok()?);
        Some(PathBuf::from(OsString::from_wide(&buffer)))
    }

    pub fn terminate(pid: u32) {
        if let Some(process) = open(pid, PROCESS_TERMINATE) {
            // SAFETY: the handle is valid for the call. An error means the process is already gone.
            let _ = unsafe { TerminateProcess(process.as_raw_handle(), 1) };
        }
    }

    pub fn ask_to_leave(pid: u32, _how: Leave) {
        terminate(pid);
    }

    pub fn kill_group(_leader: u32) {}
}

// ---------------------------------------------------------------------------------------------
// macOS and Linux
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use std::path::PathBuf;

    use super::Leave;

    unsafe extern "C" {
        /// POSIX `kill(2)`, which `std` already links. Any numbers are safe to pass: a bad pid is an error code.
        safe fn kill(pid: i32, signal: i32) -> i32;
    }

    // The same numbers on Linux and macOS.
    const SIGKILL: i32 = 9;
    const SIGTERM: i32 = 15;
    const SIGQUIT: i32 = 3;

    /// A process number that is safe to hand to `kill`: 0 would address the caller's own process group, a negative
    /// number a group, and 1 is `init`. A real service is never any of them.
    fn valid(pid: u32) -> Option<i32> {
        i32::try_from(pid).ok().filter(|&number| number > 1)
    }

    pub fn alive(pid: u32) -> bool {
        let Some(number) = valid(pid) else {
            return false;
        };
        // Signal 0 only checks that the process exists and that we may signal it.
        if kill(number, 0) != 0 {
            return false;
        }
        !is_zombie(pid)
    }

    /// A process that has ended but was not collected by its parent yet still answers signal 0.
    #[cfg(target_os = "linux")]
    fn is_zombie(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // "pid (name) S ...": the name may hold spaces and parentheses, so read after the last ')'.
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.trim_start().chars().next())
            .is_some_and(|state| matches!(state, 'Z' | 'X'))
    }

    #[cfg(not(target_os = "linux"))]
    fn is_zombie(_pid: u32) -> bool {
        false
    }

    #[cfg(target_os = "linux")]
    pub fn exe(pid: u32) -> Option<PathBuf> {
        let target = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        // A program that was replaced while running reads "<path> (deleted)".
        let text = target.to_string_lossy();
        Some(match text.strip_suffix(" (deleted)") {
            Some(path) => PathBuf::from(path),
            None => target,
        })
    }

    #[cfg(target_os = "macos")]
    pub fn exe(pid: u32) -> Option<PathBuf> {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        unsafe extern "C" {
            /// `libproc`, part of the system library: the path of the program a process runs.
            fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32;
        }
        // The documented maximum (4 * MAXPATHLEN).
        const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;

        let number = i32::try_from(pid).ok()?;
        let mut buffer = vec![0_u8; PROC_PIDPATHINFO_MAXSIZE];
        // SAFETY: the buffer is PROC_PIDPATHINFO_MAXSIZE bytes long and the call writes at most that many.
        let length = unsafe { proc_pidpath(number, buffer.as_mut_ptr().cast(), 4096) };
        let length = usize::try_from(length).ok().filter(|&n| n > 0)?;
        buffer.truncate(length);
        Some(PathBuf::from(OsString::from_vec(buffer)))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn exe(_pid: u32) -> Option<PathBuf> {
        None
    }

    fn signal(pid: u32, number: i32) {
        if let Some(pid) = valid(pid) {
            // An error means the process is already gone, which is what we wanted.
            let _ = kill(pid, number);
        }
    }

    pub fn terminate(pid: u32) {
        signal(pid, SIGKILL);
    }

    pub fn ask_to_leave(pid: u32, how: Leave) {
        signal(
            pid,
            match how {
                Leave::Terminate => SIGTERM,
                Leave::Quit => SIGQUIT,
            },
        );
    }

    pub fn kill_group(leader: u32) {
        // A negative number addresses the whole process group whose id is `leader`.
        if let Some(pid) = valid(leader) {
            let _ = kill(-pid, SIGKILL);
        }
    }
}
