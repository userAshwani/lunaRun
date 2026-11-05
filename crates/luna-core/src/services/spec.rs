//! What a service is: the program and arguments, how to know it is ready, how to stop it, how to restart it.
//!
//! A spec is plain data built by code (the PHP, MySQL and Caddy definitions arrive with the later M1.4 tasks).
//! Before the supervisor starts anything it [`validate`](ServiceSpec::validate)s the spec, and the rule that
//! matters most is golden rule 6: **a program is only ever started from inside `LUNA_HOME`**, with an argument
//! array, never through a shell.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use super::logs::OUTPUT;
use super::restart::RestartPolicy;
use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::LunaHome;

/// A short helper program that is run to ask something of a service, and that must finish: the ready check of a
/// database (`mysqladmin ping`) or its graceful stop (`mysqladmin shutdown`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// How to tell that a started process is ready to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyCheck {
    /// Something accepts a TCP connection on `127.0.0.1:<port>` (PHP, Caddy).
    Tcp { port: u16 },
    /// The helper exits with success (`mysqladmin ping` for MySQL and MariaDB).
    Command(HelperCommand),
    /// Ready as soon as the process exists (a worker that serves nothing).
    Immediately,
}

/// How a service is asked to stop before it is forced (docs/SERVICES.md "Graceful stop").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopMethod {
    /// `SIGTERM` on macOS and Linux, immediate termination on Windows (php-cgi is stateless).
    Terminate,
    /// `SIGQUIT`, the graceful stop of php-fpm. Immediate termination on Windows.
    Quit,
    /// Run a helper that makes the service shut itself down (`mysqladmin shutdown`, `caddy stop`).
    Command(HelperCommand),
    /// Terminate at once, without asking.
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPlan {
    pub method: StopMethod,
    /// How long the process gets to leave after being asked; then it is killed.
    pub grace: Duration,
}

impl Default for StopPlan {
    fn default() -> Self {
        Self {
            method: StopMethod::Terminate,
            grace: Duration::from_secs(10),
        }
    }
}

/// A log file the service writes itself (MySQL's error log, Caddy's access log), shown next to its output as a tab of
/// its own. lunaRun only reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    /// The tab's name, for example `MySQL error log`. Not `output`, which is the service's own stdout and stderr.
    pub source: String,
    /// An absolute path inside `LUNA_HOME`. The file does not have to exist yet.
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    /// `mysql@8.4`, `php-fpm@8.3.14`, `caddy`. Letters, digits and `. _ - @` only: it names files later.
    pub id: String,
    /// The program, an absolute path inside `LUNA_HOME`.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Added to the environment lunaRun itself runs in (use `php_environment` for PHP).
    pub env: Vec<(String, String)>,
    /// The working directory, inside `LUNA_HOME`. `None`: the process inherits lunaRun's own.
    pub cwd: Option<PathBuf>,
    pub ready: ReadyCheck,
    /// How long the process gets to become ready (SERVICES section 2: 30 s, 120 s for a database's first start).
    pub start_timeout: Duration,
    pub stop: StopPlan,
    pub restart: RestartPolicy,
    /// Log files the service writes itself, tailed next to its output.
    pub log_files: Vec<LogFile>,
}

impl ServiceSpec {
    /// A spec with the usual timeouts and policies: ready on a TCP port within 30 s, `SIGTERM` then kill after
    /// 10 s, three automatic restarts a minute.
    pub fn new(id: &str, program: impl Into<PathBuf>, port: u16) -> Self {
        Self {
            id: id.to_owned(),
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            ready: ReadyCheck::Tcp { port },
            start_timeout: Duration::from_secs(30),
            stop: StopPlan::default(),
            restart: RestartPolicy::default(),
            log_files: Vec::new(),
        }
    }

    /// Check everything that can be checked without starting the process. Nothing is started or changed.
    ///
    /// - the id is made of safe characters,
    /// - the program, the working directory and the helpers are absolute paths that **exist and are inside
    ///   `LUNA_HOME`** after `..` and symbolic links are resolved (`PATH_OUTSIDE_ROOT` otherwise),
    /// - the programs are files (and executable on macOS and Linux),
    /// - environment names are usable.
    ///
    /// A gap remains between this check and the launch (a file could be swapped in between). It is acceptable
    /// because only lunaRun writes inside `LUNA_HOME`, the same assumption `LunaHome::safe_remove` makes.
    pub fn validate(&self, home: &LunaHome) -> Result<()> {
        check_id(&self.id)?;
        check_program(home, &self.program, "program")?;
        if let Some(cwd) = &self.cwd {
            let resolved = inside_home(home, cwd, "working directory")?;
            if !resolved.is_dir() {
                return Err(invalid(cwd, "the working directory is not a folder"));
            }
        }
        check_env(&self.env)?;
        if let ReadyCheck::Command(helper) = &self.ready {
            check_helper(home, helper, "ready check")?;
        }
        if let StopMethod::Command(helper) = &self.stop.method {
            check_helper(home, helper, "stop command")?;
        }
        check_log_files(home, &self.log_files)
    }
}

fn invalid(path: &Path, why: &str) -> LunaError {
    LunaError::new(ErrorCode::InvalidPath, format!("{}: {why}", path.display()))
        .with_details(json!({ "path": path.display().to_string() }))
}

fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
        && !id.contains("..");
    if ok {
        Ok(())
    } else {
        Err(LunaError::internal(format!(
            "{id:?} is not a valid service id (letters, digits and . _ - @, up to 100 characters)"
        )))
    }
}

fn check_log_files(home: &LunaHome, files: &[LogFile]) -> Result<()> {
    let mut seen: Vec<&str> = vec![OUTPUT];
    for file in files {
        let name = file.source.as_str();
        if name.is_empty()
            || name.len() > 60
            || name.chars().any(char::is_control)
            || seen.contains(&name)
        {
            return Err(LunaError::internal(format!(
                "{name:?} cannot be the name of a log tab (empty, too long, a control character, `{OUTPUT}`, or used twice)"
            )));
        }
        seen.push(name);

        let path = &file.path;
        if !path.is_absolute()
            || path
                .components()
                .any(|part| part == std::path::Component::ParentDir)
        {
            return Err(invalid(
                path,
                "a log file must be an absolute path without `..`",
            ));
        }
        if path.exists() {
            // Links are followed: a link inside the data folder that points outside is outside.
            inside_home(home, path, "log file")?;
        } else if !path.starts_with(home.root()) {
            return Err(LunaError::new(
                ErrorCode::PathOutsideRoot,
                format!(
                    "refusing to read {}: the log file is not inside the lunaRun data directory",
                    path.display()
                ),
            )
            .with_details(json!({ "path": path.display().to_string() })));
        }
    }
    Ok(())
}

fn check_env(env: &[(String, String)]) -> Result<()> {
    for (name, value) in env {
        if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
            return Err(LunaError::internal(format!(
                "{name:?} cannot be used as an environment variable"
            )));
        }
    }
    Ok(())
}

fn check_helper(home: &LunaHome, helper: &HelperCommand, what: &str) -> Result<()> {
    check_program(home, &helper.program, what)?;
    check_env(&helper.env)
}

fn check_program(home: &LunaHome, program: &Path, what: &str) -> Result<()> {
    let resolved = inside_home(home, program, what)?;
    if !resolved.is_file() {
        return Err(invalid(program, &format!("the {what} is not a file")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&resolved)
            .at_path(&resolved)?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(LunaError::new(
                ErrorCode::PermissionDenied,
                format!("{} is not executable", program.display()),
            )
            .with_details(json!({ "path": program.display().to_string() })));
        }
    }
    Ok(())
}

/// `path` (absolute, existing) resolved with `..` and links followed, and proven to be strictly inside `LUNA_HOME`.
fn inside_home(home: &LunaHome, path: &Path, what: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid(
            path,
            &format!("the {what} must be an absolute path"),
        ));
    }
    let resolved = fs::canonicalize(path).at_path(path)?;
    let root = fs::canonicalize(home.root()).at_path(home.root())?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(LunaError::new(
            ErrorCode::PathOutsideRoot,
            format!(
                "refusing to start {}: the {what} is not inside the lunaRun data directory",
                path.display()
            ),
        )
        .with_details(json!({ "path": path.display().to_string() })));
    }
    Ok(resolved)
}
