//! Ad-hoc code signing of PHP on macOS (docs/RUNTIMES.md "PHP specifics").
//!
//! A downloaded binary on Apple silicon must carry at least an ad-hoc signature or the kernel kills it the
//! moment it starts. Static PHP builds are not signed (or lose their signature when unpacked), so after
//! extraction each one is checked with `codesign -v` and signed with `codesign --force --sign -` when that
//! check fails. `codesign` is an Apple system tool, the one program lunaRun starts that is not inside
//! `LUNA_HOME`, and it is started with an argument list like every other.

use std::path::Path;
use std::time::Duration;

use super::adapter::run_program;
use crate::error::Result;

/// Where macOS keeps it.
pub const CODESIGN: &str = "/usr/bin/codesign";

const TIMEOUT: Duration = Duration::from_secs(60);

/// Make sure `binary` is signed. Returns `true` if it had to be signed now.
pub fn ensure_signed(tool: &Path, binary: &Path) -> Result<bool> {
    let cwd = binary.parent().unwrap_or(binary);
    let verify = [std::ffi::OsStr::new("-v"), binary.as_os_str()];
    if run_program("php", tool, &verify, cwd, TIMEOUT).is_ok() {
        return Ok(false);
    }
    let sign = [
        std::ffi::OsStr::new("--force"),
        std::ffi::OsStr::new("--sign"),
        std::ffi::OsStr::new("-"),
        binary.as_os_str(),
    ];
    run_program("php", tool, &sign, cwd, TIMEOUT)?;
    Ok(true)
}
