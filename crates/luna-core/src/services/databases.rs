//! The database services: `mysql@<line>` and `mariadb@<line>` (FR-SV-02, FR-SV-08 to FR-SV-10; docs/SERVICES.md
//! sections 1, 2 and 4; D-027).
//!
//! [`database_definition`] turns an **installed runtime** into everything needed to run it: a ready [`ServiceSpec`]
//! (program, `--defaults-file`, port from the allocator, ready check, clean shutdown, error log as a tab) and the
//! files behind it. The config is **generated and overwritten** every time a definition is built (the port may have
//! changed); the user's own settings live in a `user.cnf` that is created once and never touched again.
//!
//! **First start** is a separate step, [`DatabaseDefinition::initialise`], that the caller runs before
//! `Supervisor::start` (D-027): it can take a minute, so it needs progress and a cancel of its own, and `start` has
//! to return at once. It does nothing when the data folder already has something in it, and it removes what it
//! made when it fails, so it can simply be tried again. The root user has **an empty password** (FR-SV-10): fine
//! for a server that is bound to `127.0.0.1` only (FR-SV-08), and the UI must say so.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::logs::last_lines;
use super::ports::{PortAllocator, PortPurpose};
use super::process::{RunEnd, run_logged};
use super::spec::{HelperCommand, LogFile, ReadyCheck, ServiceSpec, StopMethod, StopPlan};
use crate::diagnostics::Os;
use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::LunaHome;
use crate::registry::RuntimeVersion;
use crate::runtimes::InstalledRuntime;
use crate::state::write_atomic;

/// How long a database that is already set up gets to answer (SERVICES section 2).
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// A database whose data folder does not exist yet creates its tablespaces on the first start.
const FIRST_START_TIMEOUT: Duration = Duration::from_secs(120);
/// `mysqladmin shutdown` waits for the server to finish writing; then it is killed (SERVICES section 2).
const STOP_GRACE: Duration = Duration::from_secs(30);
/// How many lines of the setup program's output go into the error.
const TAIL_LINES: usize = 15;

const TEMPLATE: &str = include_str!("templates/my.ini");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Mysql,
    Mariadb,
}

impl Engine {
    pub(super) fn from_runtime(runtime: &str) -> Option<Self> {
        match runtime {
            "mysql" => Some(Self::Mysql),
            "mariadb" => Some(Self::Mariadb),
            _ => None,
        }
    }

    /// The runtime id, and the start of the service id.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Mariadb => "mariadb",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Mysql => "MySQL",
            Self::Mariadb => "MariaDB",
        }
    }
}

/// A database ready to start: the spec, where its files are, and how to set up its first start.
#[derive(Debug, Clone)]
pub struct DatabaseDefinition {
    pub engine: Engine,
    /// `8.4` for `8.4.13`: the data folder and the service belong to the line, not to the patch version.
    pub line: String,
    /// Give this to `Supervisor::start` (after [`initialise`](Self::initialise)).
    pub spec: ServiceSpec,
    pub port: u16,
    /// `data/mysql-8.4`.
    pub data_dir: PathBuf,
    /// The generated `config/mysql/8.4/my.ini` (`my.cnf` elsewhere): overwritten on every definition.
    pub config_file: PathBuf,
    /// `config/mysql/8.4/user.cnf`: the user's own settings, created once and never overwritten.
    pub user_config_file: PathBuf,
    pub error_log: PathBuf,
    /// The output of the setup program, kept for the case that it fails.
    pub init_log: PathBuf,
    /// The program that creates the data folder. Public so a test can stand in for it.
    pub init: HelperCommand,
    home: LunaHome,
}

/// Paths and settings that go into the generated config.
struct ConfigValues<'a> {
    basedir: &'a Path,
    datadir: &'a Path,
    port: u16,
    /// Unix only: a Unix socket path, which has a length limit.
    socket: Option<&'a Path>,
    error_log: &'a Path,
    user_cnf: &'a Path,
    /// MySQL 8 and later also open the X protocol on port 33060, which a second MySQL line could not have.
    mysqlx_off: bool,
}

/// A path as the option file wants it: forward slashes (they work on Windows too, and need no escaping), in quotes.
pub(super) fn quoted(path: &Path) -> Result<String> {
    Ok(format!("\"{}\"", plain(path)?))
}

/// The same without quotes (`!include` takes the rest of the line as the path).
pub(super) fn plain(path: &Path) -> Result<String> {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.contains(['"', '\n', '\r']) {
        return Err(LunaError::new(
            ErrorCode::InvalidPath,
            format!(
                "{} cannot be written into a database config (a quote or a line break in it)",
                path.display()
            ),
        )
        .with_details(json!({ "path": path.display().to_string() })));
    }
    Ok(text)
}

/// The bytes of the generated config as `engine` reads them on `os`.
///
/// MariaDB reads an option file as UTF-8. **MySQL on Windows reads it in the system's ANSI code page** (1252 on
/// most Western PCs): the UTF-8 `ü` of a folder `lüna` became two other characters, `mysqld` looked for a folder
/// that does not exist and failed its setup (found on the real MySQL 9.7). So MySQL's file is written in that
/// code page. A character the code page does not have cannot be given to `mysqld` at all (it opens files with the
/// narrow Windows calls), which is an error that says so, not a start that fails without a reason.
fn config_bytes(engine: Engine, os: Os, text: String, file: &Path) -> Result<Vec<u8>> {
    if engine != Engine::Mysql || os != Os::Windows {
        return Ok(text.into_bytes());
    }
    ansi_bytes(&text).ok_or_else(|| {
        LunaError::new(
            ErrorCode::InvalidPath,
            format!(
                "MySQL on Windows can only open folders whose names use the characters of this computer's                  code page, and {} has others: install lunaRun's data (LUNA_HOME) in a folder with a plainer name",
                file.display()
            ),
        )
        .with_details(json!({ "path": file.display().to_string() }))
    })
}

/// `text` in the system's ANSI code page, or `None` if some character is not in it.
#[cfg(windows)]
fn ansi_bytes(text: &str) -> Option<Vec<u8>> {
    use windows_sys::Win32::Globalization::{CP_ACP, WC_NO_BEST_FIT_CHARS, WideCharToMultiByte};

    if text.is_ascii() {
        return Some(text.as_bytes().to_vec());
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let wide_len = i32::try_from(wide.len()).ok()?;
    let mut used_default = 0;
    // SAFETY: `wide` is valid for `wide_len` units. With a null output and size 0 the call only measures. The
    // default-character pointer is null (the system's), and `used_default` is a valid out value.
    let size = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            wide_len,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if size <= 0 || used_default != 0 {
        return None;
    }
    let mut out = vec![0_u8; usize::try_from(size).ok()?];
    // SAFETY: `out` is valid for `size` bytes, the other arguments are as above.
    let written = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            wide_len,
            out.as_mut_ptr(),
            size,
            std::ptr::null(),
            &mut used_default,
        )
    };
    (written == size && used_default == 0).then_some(out)
}

/// Only Windows has an ANSI code page. A definition for Windows built on another system (the tests do that) gets
/// UTF-8, which is never what a real run uses: it passes `Os::current()`.
#[cfg(not(windows))]
fn ansi_bytes(text: &str) -> Option<Vec<u8>> {
    Some(text.as_bytes().to_vec())
}

fn render_config(values: &ConfigValues<'_>) -> Result<String> {
    let socket = match values.socket {
        Some(path) => format!("socket = {}\n", quoted(path)?),
        None => String::new(),
    };
    let mysqlx = if values.mysqlx_off {
        "mysqlx = OFF\n"
    } else {
        ""
    };
    Ok(TEMPLATE
        .replace("{{basedir}}", &quoted(values.basedir)?)
        .replace("{{datadir}}", &quoted(values.datadir)?)
        .replace("{{port}}", &values.port.to_string())
        .replace("{{socket}}", &socket)
        .replace("{{error_log}}", &quoted(values.error_log)?)
        .replace("{{mysqlx}}\n", mysqlx)
        .replace("{{user_cnf}}", &plain(values.user_cnf)?))
}

/// FNV-1a: a short stable number for a path (the Unix socket fallback name).
fn fnv(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// Where the Unix socket goes. `run/mysql-8.4.sock` in the data folder, unless that is too long for a socket path
/// (about 104 bytes on macOS, 108 on Linux): then a short name in the system's temp folder, made unique by a hash
/// of the data folder.
fn socket_path(home: &LunaHome, engine: Engine, line: &str) -> PathBuf {
    let name = format!("{}-{line}.sock", engine.name());
    let preferred = home.run_dir().join(&name);
    if preferred.as_os_str().len() <= 100 {
        return preferred;
    }
    let tag = fnv(&home.root().to_string_lossy());
    std::env::temp_dir().join(format!("lunarun-{tag:08x}-{name}"))
}

/// Build the definition of one installed database, and write its generated config.
///
/// - The service id is `mysql@<line>` or `mariadb@<line>` (`8.4` for `8.4.13`).
/// - The port comes from the allocator and is kept for next time ([`PortAllocator::assign`]); the spec checks it
///   before every start.
/// - `os` is a parameter so that both variants (Windows `my.ini` and `.exe`, Unix `my.cnf` and a socket) are tested
///   on any machine. Pass `Os::current()`.
///
/// # Errors
/// `INTERNAL` for a runtime that is not a database or a version without numbers, `INVALID_PATH` for a path that
/// cannot be written into a config, `NO_FREE_PORT`/`PORT_IN_USE` from the allocator, and I/O errors.
pub fn database_definition(
    home: &LunaHome,
    ports: &PortAllocator,
    runtime: &InstalledRuntime,
    os: Os,
) -> Result<DatabaseDefinition> {
    let engine = Engine::from_runtime(&runtime.runtime)
        .ok_or_else(|| LunaError::internal(format!("{} is not a database", runtime.runtime)))?;
    let version = RuntimeVersion::parse(&runtime.version).ok_or_else(|| {
        LunaError::internal(format!("{:?} is not a version number", runtime.version))
    })?;
    let line = version.major_minor();
    let major: u32 = line
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    let id = format!("{}@{line}", engine.name());
    let port = ports.assign(&id, PortPurpose::Database)?.port;

    let data_dir = data_dir_of(home, engine, &line);
    let config_dir = home.config_dir().join(engine.name()).join(&line);
    let config_file = config_dir.join(if os == Os::Windows {
        "my.ini"
    } else {
        "my.cnf"
    });
    let user_config_file = config_dir.join("user.cnf");
    let error_log = home
        .logs_dir()
        .join(format!("{}-{line}.err", engine.name()));
    let init_log = home
        .logs_dir()
        .join(format!("{}-{line}.init.log", engine.name()));
    let socket = (os != Os::Windows).then(|| socket_path(home, engine, &line));

    // The generated config is rewritten every time; the user's file only when it is not there yet.
    fs::create_dir_all(home.logs_dir()).at_path(&home.logs_dir())?;
    if socket
        .as_deref()
        .is_some_and(|s| s.starts_with(home.run_dir()))
    {
        fs::create_dir_all(home.run_dir()).at_path(&home.run_dir())?;
    }
    let text = render_config(&ConfigValues {
        basedir: &runtime.dir,
        datadir: &data_dir,
        port,
        socket: socket.as_deref(),
        error_log: &error_log,
        user_cnf: &user_config_file,
        mysqlx_off: engine == Engine::Mysql && major >= 8,
    })?;
    write_atomic(&config_file, &config_bytes(engine, os, text, &config_file)?)?;
    if !user_config_file.exists() {
        let header = format!(
            "# Your own {} settings. lunaRun never overwrites this file.\n[mysqld]\n",
            engine.label()
        );
        write_atomic(&user_config_file, header.as_bytes())?;
    }

    let bin = runtime.dir.join("bin");
    let exe = |name: &str| {
        bin.join(if os == Os::Windows {
            format!("{name}.exe")
        } else {
            name.to_owned()
        })
    };
    let (server, admin) = match engine {
        Engine::Mysql => (exe("mysqld"), exe("mysqladmin")),
        Engine::Mariadb => (exe("mariadbd"), exe("mariadb-admin")),
    };
    let defaults_file = format!("--defaults-file={}", config_file.display());
    let admin_command = |verb: &str| HelperCommand {
        program: admin.clone(),
        // `--no-defaults` first: do not read a my.ini of some other MySQL (XAMPP's) from the system.
        args: [
            "--no-defaults",
            "--protocol=TCP",
            "-h",
            "127.0.0.1",
            "-P",
            &port.to_string(),
            "-u",
            "root",
            verb,
        ]
        .map(str::to_owned)
        .to_vec(),
        env: Vec::new(),
    };

    let init = match engine {
        Engine::Mysql => HelperCommand {
            program: server.clone(),
            args: vec![defaults_file.clone(), "--initialize-insecure".to_owned()],
            env: Vec::new(),
        },
        Engine::Mariadb => HelperCommand {
            program: exe("mariadb-install-db"),
            args: if os == Os::Windows {
                vec![
                    format!("--datadir={}", data_dir.display()),
                    "--silent".to_owned(),
                ]
            } else {
                vec![
                    defaults_file.clone(),
                    format!("--basedir={}", runtime.dir.display()),
                    format!("--datadir={}", data_dir.display()),
                    // MariaDB 10.4 and later give root a login through the Unix socket only by default: a
                    // password-less root over TCP, as a local Laravel needs, takes this.
                    "--auth-root-authentication-method=normal".to_owned(),
                    "--skip-test-db".to_owned(),
                ]
            },
            env: Vec::new(),
        },
    };

    let first_start = data_dir_is_empty(&data_dir);
    let mut spec = ServiceSpec::new(&id, server, port);
    spec.args = vec![defaults_file];
    spec.ready = ReadyCheck::Command(admin_command("ping"));
    spec.stop = StopPlan {
        method: StopMethod::Command(admin_command("shutdown")),
        grace: STOP_GRACE,
    };
    spec.start_timeout = if first_start {
        FIRST_START_TIMEOUT
    } else {
        START_TIMEOUT
    };
    spec.log_files = vec![LogFile {
        source: format!("{} error log", engine.label()),
        path: error_log.clone(),
    }];

    Ok(DatabaseDefinition {
        engine,
        line,
        spec,
        port,
        data_dir,
        config_file,
        user_config_file,
        error_log,
        init_log,
        init,
        home: home.clone(),
    })
}

/// `data/mysql-8.4`: the data folder belongs to the line, and the patch versions of a line share it.
pub(super) fn data_dir_of(home: &LunaHome, engine: Engine, line: &str) -> PathBuf {
    home.data_dir().join(format!("{}-{line}", engine.name()))
}

/// Missing, or there but with nothing in it: the database has never been set up.
pub(super) fn data_dir_is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).map_or(true, |mut entries| entries.next().is_none())
}

impl DatabaseDefinition {
    /// The data folder is missing or empty, so the database must be set up before it can start.
    pub fn needs_initialisation(&self) -> bool {
        data_dir_is_empty(&self.data_dir)
    }

    /// Set the database up for its first start (FR-SV-10), if it needs it: run the engine's own setup program and
    /// check that it made a data folder. Does nothing when the data folder already has something in it, so it is safe
    /// to call before every start.
    ///
    /// Gives up after `limit` and when `cancel` fires. In every failure, what was made is removed, so the next try
    /// starts clean; the error is `DATABASE_INIT_FAILED` (or `CANCELLED`) and carries the end of the setup
    /// program's output, which is also in `init_log`.
    pub async fn initialise(&self, limit: Duration, cancel: &CancellationToken) -> Result<()> {
        if !self.needs_initialisation() {
            return Ok(());
        }
        fs::create_dir_all(self.home.logs_dir()).at_path(&self.home.logs_dir())?;
        let log = File::create(&self.init_log).at_path(&self.init_log)?;

        let outcome = run_logged(&self.init, &log, limit, cancel).await;
        let failure = match &outcome {
            Ok(RunEnd::Exited(Some(0))) if !self.needs_initialisation() => return Ok(()),
            Ok(RunEnd::Exited(Some(0))) => {
                "the setup program finished but made no data folder".to_owned()
            }
            Ok(RunEnd::Exited(Some(code))) => {
                format!("the setup program failed with exit code {code}")
            }
            Ok(RunEnd::Exited(None)) => "the setup program was ended before it finished".to_owned(),
            Ok(RunEnd::TimedOut) => format!(
                "the setup program did not finish within {} s",
                limit.as_secs()
            ),
            Ok(RunEnd::Cancelled) => String::new(),
            Err(err) => format!("the setup program could not be started: {err}"),
        };

        // Whatever it made is partial: MySQL refuses a data folder that has files in it, so a retry needs it gone.
        let _ = self.home.safe_remove(&self.data_dir);
        if matches!(outcome, Ok(RunEnd::Cancelled)) {
            return Err(LunaError::new(
                ErrorCode::Cancelled,
                format!(
                    "setting up {} {} was cancelled",
                    self.engine.label(),
                    self.line
                ),
            ));
        }
        let tail = last_lines(&self.init_log, TAIL_LINES, 500);
        Err(LunaError::new(
            ErrorCode::DatabaseInitFailed,
            format!("{} {}: {failure}", self.engine.label(), self.line),
        )
        .with_details(json!({
            "service": self.spec.id,
            "log": self.init_log.display().to_string(),
            "tail": tail,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values<'a>(socket: Option<&'a Path>, mysqlx_off: bool) -> ConfigValues<'a> {
        ConfigValues {
            basedir: Path::new(r"D:\lüna home\runtimes\mysql\8.4.13"),
            datadir: Path::new(r"D:\lüna home\data\mysql-8.4"),
            port: 3307,
            socket,
            error_log: Path::new(r"D:\lüna home\logs\mysql-8.4.err"),
            user_cnf: Path::new(r"D:\lüna home\config\mysql\8.4\user.cnf"),
            mysqlx_off,
        }
    }

    #[test]
    fn only_mysql_on_windows_gets_its_config_in_the_ansi_code_page() {
        let file = Path::new("my.ini");
        let text = "datadir = \"D:/lüna home\"\n";
        // MariaDB, and both engines on macOS and Linux, read UTF-8.
        for (engine, os) in [
            (Engine::Mariadb, Os::Windows),
            (Engine::Mariadb, Os::Linux),
            (Engine::Mysql, Os::Linux),
            (Engine::Mysql, Os::Macos),
        ] {
            assert_eq!(
                config_bytes(engine, os, text.to_owned(), file).unwrap(),
                text.as_bytes(),
                "{engine:?} {os:?}"
            );
        }
        // Plain ASCII is the same in every encoding.
        assert_eq!(
            config_bytes(Engine::Mysql, Os::Windows, "port = 1\n".to_owned(), file).unwrap(),
            b"port = 1\n"
        );
    }

    #[cfg(windows)]
    #[test]
    fn mysql_on_windows_gets_the_system_code_page_and_a_clear_error_for_what_it_lacks() {
        use windows_sys::Win32::Globalization::GetACP;

        // Western European code page: ü is the single byte 0xFC, not the two bytes of UTF-8.
        // SAFETY: `GetACP` takes no arguments and only reads the system setting.
        if unsafe { GetACP() } == 1252 {
            let bytes = config_bytes(
                Engine::Mysql,
                Os::Windows,
                "a = \"lüna\"".to_owned(),
                Path::new("my.ini"),
            )
            .unwrap();
            assert_eq!(bytes, b"a = \"l\xFCna\"");
        }
        // No ANSI code page has an emoji.
        let err = config_bytes(
            Engine::Mysql,
            Os::Windows,
            "a = \"\u{1F600}\"".to_owned(),
            Path::new("C:/x/my.ini"),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidPath);
        assert!(err.message.contains("code page"), "{}", err.message);
    }

    #[test]
    fn the_config_has_every_setting_with_forward_slashes_and_quotes_where_the_option_file_wants_them()
     {
        let text = render_config(&values(None, true)).unwrap();

        assert!(text.contains("[mysqld]\n"));
        assert!(text.contains("basedir = \"D:/lüna home/runtimes/mysql/8.4.13\"\n"));
        assert!(text.contains("datadir = \"D:/lüna home/data/mysql-8.4\"\n"));
        assert!(text.contains("port = 3307\n"));
        assert!(
            text.contains("bind-address = 127.0.0.1\n"),
            "FR-SV-08: only this computer"
        );
        assert!(text.contains("log-error = \"D:/lüna home/logs/mysql-8.4.err\"\n"));
        assert!(text.contains("character-set-server = utf8mb4\n"));
        assert!(text.contains("collation-server = utf8mb4_unicode_ci\n"));
        assert!(text.contains("max_allowed_packet = 64M\n"));
        assert!(text.contains("mysqlx = OFF\n"));
        assert!(!text.contains("socket"), "no socket on Windows");
        assert!(!text.contains('\\'), "no backslash anywhere: {text}");
        assert!(!text.contains("{{"), "every placeholder was filled: {text}");
        // The user's file is included last, unquoted (the rest of the line is the path).
        assert!(
            text.trim_end()
                .ends_with("!include D:/lüna home/config/mysql/8.4/user.cnf"),
            "{text}"
        );
    }

    #[test]
    fn a_unix_config_has_a_socket_and_mariadb_has_no_x_protocol_line() {
        let socket = Path::new("/home/me/lüna home/run/mariadb-11.8.sock");
        let text = render_config(&values(Some(socket), false)).unwrap();

        assert!(text.contains("socket = \"/home/me/lüna home/run/mariadb-11.8.sock\"\n"));
        assert!(!text.contains("mysqlx"));
    }

    #[test]
    fn a_path_that_cannot_be_written_into_the_config_is_refused() {
        for bad in ["a\"b", "a\nb", "a\rb"] {
            let mut v = values(None, false);
            let path = PathBuf::from(format!("/data/{bad}"));
            v.datadir = &path;
            let error = render_config(&v).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidPath, "{bad:?}");
        }
    }

    #[test]
    fn a_long_socket_path_falls_back_to_a_short_one_in_the_temp_folder() {
        let tmp = tempfile::TempDir::new().unwrap();
        let short = LunaHome::at(tmp.path().join("home")).unwrap();
        assert_eq!(
            socket_path(&short, Engine::Mysql, "8.4"),
            short.run_dir().join("mysql-8.4.sock")
        );

        let long = LunaHome::at(tmp.path().join("a".repeat(120)).join("home")).unwrap();
        let fallback = socket_path(&long, Engine::Mysql, "8.4");
        assert!(
            fallback.starts_with(std::env::temp_dir()),
            "{}",
            fallback.display()
        );
        assert!(fallback.as_os_str().len() <= 100, "{}", fallback.display());
        // The same home always gets the same name, and another home another one.
        assert_eq!(fallback, socket_path(&long, Engine::Mysql, "8.4"));
        let other = LunaHome::at(tmp.path().join("b".repeat(120)).join("home")).unwrap();
        assert_ne!(fallback, socket_path(&other, Engine::Mysql, "8.4"));
    }
}
