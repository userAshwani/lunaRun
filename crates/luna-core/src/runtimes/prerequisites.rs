//! Things a runtime needs that lunaRun cannot install for it (FR-RT-12). Today that is one: the Microsoft
//! Visual C++ 2015-2022 Redistributable (x64) on Windows, which PHP, MySQL and MariaDB are built against.
//! Without it the freshly installed program does not even start, which is a confusing way to learn it, so the
//! install is refused up front with a message and a download address.
//!
//! The registry names a prerequisite in the `requires` list of an artifact (`vcredist-14-x64`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use super::adapter::run_program_raw;
use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};

/// The registry's name for the redistributable.
pub const VCREDIST_ID: &str = "vcredist-14-x64";

/// What users see.
pub const VCREDIST_NAME: &str = "Microsoft Visual C++ 2015-2022 Redistributable (x64)";

/// Microsoft's permanent address for the latest 2015-2022 x64 redistributable. It redirects to the real
/// download and always serves the newest build.
pub const VCREDIST_DOWNLOAD_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

/// The oldest redistributable (`major.minor`) accepted: **14.40**, the runtime of Visual Studio 2022 17.10.
///
/// The 2015, 2017, 2019 and 2022 runtimes share one version line (14.x), and an old one reports
/// `Installed = 1` while `php.exe` still fails to start because the program was built against a newer
/// toolset. Microsoft promises that a newer runtime runs older programs, never the other way round. 14.40 is
/// where the C++ standard library last changed in a way that makes programs built with newer toolsets
/// (current PHP, MySQL and MariaDB builds) crash or refuse to start on an older runtime. Every
/// redistributable published since May 2024 satisfies it, and updating one is free. Raise it if a runtime
/// registered later needs more.
pub const VCREDIST_MINIMUM: (u32, u32) = (14, 40);

const REG_KEY: &str = r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64";
const REG_TIMEOUT: Duration = Duration::from_secs(15);

/// What the registry says about the installed redistributable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcRedist {
    /// The `Installed` value is 1.
    pub installed: bool,
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    /// Microsoft's own text, such as `v14.51.36247.00`.
    pub version: Option<String>,
}

impl VcRedist {
    /// New enough for [`VCREDIST_MINIMUM`].
    pub fn is_recent_enough(&self) -> bool {
        (self.major, self.minor) >= VCREDIST_MINIMUM
    }

    /// `14.51.36247` (or Microsoft's text when it is there), for messages.
    pub fn shown_version(&self) -> String {
        self.version
            .as_deref()
            .map(|v| v.trim_start_matches('v').to_owned())
            .unwrap_or_else(|| format!("{}.{}.{}", self.major, self.minor, self.build))
    }
}

/// A `0x1f` or `31` as `reg query` prints a `REG_DWORD`.
fn dword(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// Read the output of `reg query <key>`. Values are found by their registry names and `REG_*` type words,
/// which do not change with the Windows display language (error messages do, so they are never read).
/// `None` when the output has no `Installed` value at all.
pub fn parse_reg_output(output: &str) -> Option<VcRedist> {
    let mut installed = None;
    let (mut major, mut minor, mut build) = (0, 0, 0);
    let mut version = None;
    for line in output.lines() {
        let mut words = line.split_whitespace();
        let (Some(name), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        let value = words.next();
        match (name, kind, value) {
            ("Installed", "REG_DWORD", Some(v)) => installed = dword(v),
            ("Major", "REG_DWORD", Some(v)) => major = dword(v).unwrap_or(0),
            ("Minor", "REG_DWORD", Some(v)) => minor = dword(v).unwrap_or(0),
            ("Bld", "REG_DWORD", Some(v)) => build = dword(v).unwrap_or(0),
            ("Version", "REG_SZ", Some(v)) => version = Some(v.to_owned()),
            _ => {}
        }
    }
    Some(VcRedist {
        installed: installed? == 1,
        major,
        minor,
        build,
        version,
    })
}

/// Where Windows keeps `reg.exe`.
pub fn default_reg_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join("reg.exe")
}

/// Ask the registry (through `reg query`, an argument list, no shell and no extra crate) about the
/// redistributable. `Ok(None)`: the key does not exist, so it is not installed. `Err`: the answer could not be
/// obtained (`reg` missing, hung) and nothing is known.
pub fn detect_vcredist(reg: &Path) -> Result<Option<VcRedist>> {
    let cwd = reg.parent().unwrap_or(reg);
    let finished = run_program_raw("vcredist", reg, &["query", REG_KEY], cwd, REG_TIMEOUT)?;
    if !finished.success {
        // `reg` exits with 1 for a key that is not there.
        return Ok(None);
    }
    Ok(parse_reg_output(&finished.output))
}

/// A requirement is missing or too old.
fn missing(found: Option<&VcRedist>, why: String) -> LunaError {
    LunaError::new(ErrorCode::PrerequisiteMissing, why).with_details(json!({
        "requirement": VCREDIST_ID,
        "name": VCREDIST_NAME,
        "downloadUrl": VCREDIST_DOWNLOAD_URL,
        "minimum": format!("{}.{}", VCREDIST_MINIMUM.0, VCREDIST_MINIMUM.1),
        "found": found.map(VcRedist::shown_version),
    }))
}

/// Refuse the install when something in `requires` is not satisfied on `os`.
///
/// - Only Windows has prerequisites; on macOS and Linux the registry names none, and nothing is checked.
/// - A requirement this build does not know is refused, never ignored: a registry that asks for something
///   lunaRun cannot check must not silently produce a runtime that does not start.
/// - If the registry cannot be read at all (`detect` fails), the install is **not** blocked: a wrong "missing"
///   is worse than a program that fails to start with a clear `RUNTIME_BROKEN` message, and it is the same
///   rule as for an unreadable free-disk figure.
pub fn check_requirements(
    os: Os,
    requires: &[String],
    detect: &dyn Fn() -> Result<Option<VcRedist>>,
) -> Result<()> {
    if os != Os::Windows {
        return Ok(());
    }
    for requirement in requires {
        if requirement != VCREDIST_ID {
            return Err(LunaError::new(
                ErrorCode::PrerequisiteMissing,
                format!("this version of lunaRun does not know the requirement {requirement:?}"),
            )
            .with_details(json!({ "requirement": requirement })));
        }
        let Ok(found) = detect() else {
            continue;
        };
        match found {
            None => {
                return Err(missing(None, format!("{VCREDIST_NAME} is not installed")));
            }
            Some(redist) if !redist.installed => {
                return Err(missing(
                    Some(&redist),
                    format!("{VCREDIST_NAME} is not installed"),
                ));
            }
            Some(redist) if !redist.is_recent_enough() => {
                return Err(missing(
                    Some(&redist),
                    format!(
                        "{VCREDIST_NAME} {} is too old, {}.{} or newer is needed",
                        redist.shown_version(),
                        VCREDIST_MINIMUM.0,
                        VCREDIST_MINIMUM.1
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// The check the installer uses unless told otherwise: the real registry on Windows, read through `reg`
/// (normally [`default_reg_path`]).
pub fn default_check(os: Os, requires: &[String], reg: &Path) -> Result<()> {
    check_requirements(os, requires, &|| detect_vcredist(reg))
}
