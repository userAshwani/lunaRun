//! The configuration lunaRun generates for every PHP version it installs (FR-RT-09, docs/SERVICES.md
//! section 4 "php.ini").
//!
//! Generated files live in `LUNA_HOME/config/php/<version>/` and are **owned by lunaRun and rewritten at
//! every install**: `php.ini` there, plus the one CA bundle shared by all versions
//! (`config/php/cacert.pem`). The user's own settings go into `conf.d/99-user.ini`, which is created once
//! and **never overwritten**. Every PHP process lunaRun starts gets [`php_environment`], which points PHP at
//! both.
//!
//! The Windows `php.ini` starts from the zip's own `php.ini-development` (good development defaults) and
//! changes only the lines it means to: [`render_windows_ini`] edits in place and keeps everything else,
//! including the file's line endings. Static builds (macOS, Linux) have no such file and cannot load
//! extensions, so they get a small template.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::{LunaHome, component};
use crate::state::write_atomic;

/// Mozilla's trusted certificate authorities, as published by curl (MPL-2.0). Windows PHP has no certificate
/// store of its own, so without this Composer and every HTTPS call from PHP fail with SSL errors. Refresh it
/// with `pnpm ca-bundle`.
pub const CA_BUNDLE: &str = include_str!("../../data/cacert.pem");

const STATIC_TEMPLATE: &str = include_str!("templates/php-static.ini");

/// The extensions lunaRun switches on in a Windows PHP: what Laravel and the usual packages need, plus the
/// database drivers. Only those whose `php_<name>.dll` is really in the zip are enabled.
pub const WINDOWS_EXTENSIONS: [&str; 20] = [
    "bz2",
    "curl",
    "exif",
    "fileinfo",
    "ftp",
    "gd",
    "gmp",
    "intl",
    "mbstring",
    "mysqli",
    "openssl",
    "pdo_mysql",
    "pdo_pgsql",
    "pdo_sqlite",
    "pgsql",
    "soap",
    "sockets",
    "sodium",
    "sqlite3",
    "zip",
];

/// What the generated `php.ini` needs to know.
#[derive(Debug, Clone)]
pub struct PhpSettings<'a> {
    /// Windows only: the folder with `php_*.dll`, in the **final** install folder (not the staging one).
    pub extension_dir: &'a Path,
    /// The CA bundle every PHP version shares.
    pub cafile: &'a Path,
    /// An IANA name such as `Europe/Berlin`, or `UTC`.
    pub timezone: &'a str,
    /// Names of the extensions that exist: `curl` for `ext/php_curl.dll`. Windows only.
    pub available_extensions: &'a BTreeSet<String>,
}

/// A rendered Windows `php.ini` and what was and was not switched on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedIni {
    pub text: String,
    /// Extensions switched on, in the order they appear in the file.
    pub enabled: Vec<String>,
    /// Wanted extensions this PHP does not have.
    pub missing: Vec<String>,
}

/// Where a PHP version's generated and user files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpConfigPaths {
    /// `config/php/<version>`
    pub dir: PathBuf,
    pub ini: PathBuf,
    /// Loaded after `php.ini` through `PHP_INI_SCAN_DIR`.
    pub conf_d: PathBuf,
    /// Created once, never overwritten.
    pub user_ini: PathBuf,
    /// Shared by all versions.
    pub cacert: PathBuf,
}

pub fn php_config_paths(home: &LunaHome, version: &str) -> Result<PhpConfigPaths> {
    let php = home.config_dir().join("php");
    let dir = php.join(component(version)?);
    let conf_d = dir.join("conf.d");
    Ok(PhpConfigPaths {
        ini: dir.join("php.ini"),
        user_ini: conf_d.join("99-user.ini"),
        conf_d,
        cacert: php.join("cacert.pem"),
        dir,
    })
}

/// The environment every PHP process of this version must be started with, so that it reads the generated
/// `php.ini` and the user's `conf.d` (services in M1.4, the terminal in M1.7).
pub fn php_environment(home: &LunaHome, version: &str) -> Result<Vec<(&'static str, OsString)>> {
    let paths = php_config_paths(home, version)?;
    Ok(vec![
        ("PHPRC", paths.dir.into_os_string()),
        ("PHP_INI_SCAN_DIR", paths.conf_d.into_os_string()),
    ])
}

// ---------------------------------------------------------------------------------------------
// Editing an INI file in place
// ---------------------------------------------------------------------------------------------

/// A PHP ini file as lines, edited without disturbing the rest of the file.
struct IniEdit {
    lines: Vec<String>,
    eol: &'static str,
    trailing_newline: bool,
}

/// The key of a `key = value` line, with a leading `;` allowed.
fn key_of(line: &str, commented: bool) -> Option<&str> {
    let mut rest = line.trim_start();
    if commented {
        rest = rest.strip_prefix(';')?.trim_start();
    }
    let end = rest.find(|c: char| c == '=' || c.is_whitespace())?;
    let (key, after) = rest.split_at(end);
    (!key.is_empty() && after.trim_start().starts_with('=')).then_some(key)
}

impl IniEdit {
    fn new(text: &str) -> Self {
        Self {
            eol: if text.contains("\r\n") { "\r\n" } else { "\n" },
            trailing_newline: text.ends_with('\n'),
            lines: text.lines().map(str::to_owned).collect(),
        }
    }

    /// Make `key = value` the setting: replace the first active line of that key, else the first
    /// commented-out one, else add it at the end. (PHP applies a plain key from any section.)
    fn set(&mut self, key: &str, value: &str) {
        let line = format!("{key} = {value}");
        let at = self
            .lines
            .iter()
            .position(|l| key_of(l, false) == Some(key))
            .or_else(|| self.lines.iter().position(|l| key_of(l, true) == Some(key)));
        match at {
            Some(index) => self.lines[index] = line,
            None => self.lines.push(line),
        }
    }

    /// Switch on `directive=name` (`extension` or `zend_extension`): uncomment its line in place so the
    /// file's own load order is kept (`exif` must come after `mbstring`), or add it at the end.
    fn enable(&mut self, directive: &str, name: &str) {
        let wanted = format!("{directive}={name}");
        let matches = |line: &str, commented: bool| {
            let rest = if commented {
                match line.trim_start().strip_prefix(';') {
                    Some(rest) => rest.trim_start(),
                    None => return false,
                }
            } else {
                line.trim_start()
            };
            rest.strip_prefix(&wanted).is_some_and(|after| {
                after.is_empty() || after.starts_with(char::is_whitespace) || after.starts_with(';')
            })
        };
        if self.lines.iter().any(|l| matches(l, false)) {
            return;
        }
        match self.lines.iter().position(|l| matches(l, true)) {
            Some(index) => self.lines[index] = wanted,
            None => self.lines.push(wanted),
        }
    }

    fn finish(self) -> String {
        let mut text = self.lines.join(self.eol);
        if self.trailing_newline {
            text.push_str(self.eol);
        }
        text
    }
}

/// A path as an INI string value. Quoted, and refused when PHP would read it differently from what we mean:
/// a quote ends the string early and `${` starts an expansion, neither of which has an escape.
fn quoted(path: &Path) -> Result<String> {
    let text = path.to_string_lossy();
    if text.contains('"') || text.contains("${") || text.contains(['\n', '\r']) {
        return Err(LunaError::new(
            ErrorCode::InvalidPath,
            format!(
                "{text:?} cannot be written into php.ini safely (it contains a quote, a line break or ${{)"
            ),
        ));
    }
    Ok(format!("\"{text}\""))
}

/// The Windows `php.ini`: `base` (the zip's `php.ini-development`) with lunaRun's settings applied.
pub fn render_windows_ini(base: &str, settings: &PhpSettings<'_>) -> Result<RenderedIni> {
    let mut ini = IniEdit::new(base);
    ini.set("extension_dir", &quoted(settings.extension_dir)?);
    apply_common(&mut ini, settings)?;

    // Opcache is a Zend extension.
    ini.set("opcache.enable", "1");
    ini.set("opcache.validate_timestamps", "1");
    ini.set("opcache.revalidate_freq", "0");

    let have = |name: &str| settings.available_extensions.contains(name);
    let mut enabled = Vec::new();
    let mut missing = Vec::new();
    for name in WINDOWS_EXTENSIONS {
        if have(name) {
            ini.enable("extension", name);
            enabled.push(name.to_owned());
        } else {
            missing.push(name.to_owned());
        }
    }
    if have("opcache") {
        ini.enable("zend_extension", "opcache");
        enabled.push("opcache".to_owned());
    } else {
        missing.push("opcache".to_owned());
    }

    // Report in the order the file loads them.
    let text = ini.finish();
    let position = |name: &String| {
        text.lines()
            .position(|l| {
                let l = l.trim_start();
                l.starts_with(&format!("extension={name}"))
                    || l.starts_with(&format!("zend_extension={name}"))
            })
            .unwrap_or(usize::MAX)
    };
    enabled.sort_by_key(position);
    Ok(RenderedIni {
        text,
        enabled,
        missing,
    })
}

fn apply_common(ini: &mut IniEdit, settings: &PhpSettings<'_>) -> Result<()> {
    ini.set("memory_limit", "512M");
    ini.set("upload_max_filesize", "128M");
    ini.set("post_max_size", "128M");
    ini.set("max_execution_time", "120");
    ini.set("date.timezone", &format!("\"{}\"", settings.timezone));
    let cafile = quoted(settings.cafile)?;
    ini.set("curl.cainfo", &cafile);
    ini.set("openssl.cafile", &cafile);
    Ok(())
}

/// The `php.ini` of a static build (macOS, Linux).
pub fn render_static_ini(settings: &PhpSettings<'_>) -> Result<String> {
    Ok(STATIC_TEMPLATE
        .replace("{{timezone}}", &format!("\"{}\"", settings.timezone))
        .replace("{{cafile}}", &quoted(settings.cafile)?))
}

// ---------------------------------------------------------------------------------------------
// Time zone
// ---------------------------------------------------------------------------------------------

/// A valid IANA zone name, else `UTC`. Only letters, digits and `_ + - /` are allowed (so no `.`, hence no
/// `..`, and nothing odd can end up inside a quoted ini value).
fn clean_zone(name: &str) -> Option<String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'));
    valid.then(|| name.to_owned())
}

/// The zone from what a Unix system provides: the target of the `/etc/localtime` link
/// (`.../zoneinfo/Europe/Berlin`), else the text of `/etc/timezone`. `UTC` when neither says.
pub fn timezone_from(localtime_target: Option<&Path>, etc_timezone: Option<&str>) -> String {
    let from_link = localtime_target.and_then(|target| {
        let text = target.to_string_lossy().replace('\\', "/");
        let (_, zone) = text.rsplit_once("zoneinfo/")?;
        clean_zone(zone)
    });
    from_link
        .or_else(|| etc_timezone.and_then(clean_zone))
        .unwrap_or_else(|| "UTC".to_owned())
}

/// This computer's time zone. Windows names its zones differently from the IANA names PHP wants, and
/// Laravel sets its own zone (`APP_TIMEZONE`, default UTC) anyway, so Windows gets `UTC`.
pub fn system_timezone() -> String {
    if cfg!(windows) {
        return "UTC".to_owned();
    }
    let link = fs::read_link("/etc/localtime").ok();
    let file = fs::read_to_string("/etc/timezone").ok();
    timezone_from(link.as_deref(), file.as_deref())
}

// ---------------------------------------------------------------------------------------------
// Writing it out
// ---------------------------------------------------------------------------------------------

const USER_INI_HEADER: &str = "; Your own PHP settings for this version. lunaRun creates this file once and never overwrites it.\n\
; Example:\n\
;   memory_limit = 1G\n";

/// Write everything a PHP version needs on disk: the shared CA bundle, `php.ini`, and the user's `conf.d`
/// with a `99-user.ini` that is only created when it does not exist yet.
pub fn write_php_config(paths: &PhpConfigPaths, ini_text: &str) -> Result<()> {
    write_atomic(&paths.cacert, CA_BUNDLE.as_bytes())?;
    write_atomic(&paths.ini, ini_text.as_bytes())?;
    fs::create_dir_all(&paths.conf_d).at_path(&paths.conf_d)?;
    if !paths.user_ini.exists() {
        write_atomic(&paths.user_ini, USER_INI_HEADER.as_bytes())?;
    }
    Ok(())
}

/// Names of the `php_<name>.dll` files in `ext_dir`.
pub fn available_extensions(ext_dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(ext_dir) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let name = name.strip_prefix("php_")?.strip_suffix(".dll")?;
            Some(name.to_owned())
        })
        .collect()
}
