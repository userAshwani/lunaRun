//! The PHP services: `php-fcgi@<version>` on Windows and `php-fpm@<version>` on macOS and Linux (FR-SV-02;
//! docs/SERVICES.md sections 1, 2 and 4; D-028).
//!
//! [`php_definition`] turns an **installed PHP** into the specs that run it and the FastCGI ports Caddy sends PHP
//! requests to. A pool is identified by the **exact** version (`php-fpm@8.3.36`), so several versions run side by side.
//!
//! - **Windows** has no `php-fpm`: `php-cgi.exe` serves one request at a time on one port, so a pool is `workers`
//!   processes on consecutive ports and Caddy spreads requests over them. The supervisor owns one process per
//!   service, so each worker is a service of its own (`php-fcgi@8.3.36-w1` and up) that is started, stopped, restarted
//!   and cleaned up like any other. `php-cgi` exits by design after `PHP_FCGI_MAX_REQUESTS` requests, which
//!   [`RestartPolicy::recycle_on_exit`](super::RestartPolicy) restarts at once without calling it a crash.
//! - **macOS and Linux** run one `php-fpm` in the foreground that manages its own children, with a generated
//!   `php-fpm.conf` (one pool on `127.0.0.1:<port>`).

use std::path::PathBuf;
use std::time::Duration;

use super::databases::quoted;
use super::ports::{PortAllocator, PortPurpose};
use super::spec::{LogFile, ServiceSpec, StopMethod, StopPlan};
use crate::diagnostics::Os;
use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::LunaHome;
use crate::runtimes::{InstalledRuntime, php_config_paths, php_environment};
use crate::state::write_atomic;

/// How many `php-cgi` processes a Windows pool has unless the user says otherwise (SERVICES section 1).
pub const DEFAULT_WORKERS: u16 = 4;
/// More than this is never useful on a development machine and would use up the port range.
pub const MAX_WORKERS: u16 = 16;
/// `php-cgi` exits after this many requests (SERVICES section 2); it is started again at once.
const MAX_REQUESTS: &str = "500";
/// `php-fpm` finishes the requests it is serving on `SIGQUIT`; after this it is killed.
const FPM_STOP_GRACE: Duration = Duration::from_secs(10);

const TEMPLATE: &str = include_str!("templates/php-fpm.conf");

/// `php-fcgi` on Windows, `php-fpm` elsewhere: the start of a pool's id.
pub(super) fn pool_kind(os: Os) -> &'static str {
    if os == Os::Windows {
        "php-fcgi"
    } else {
        "php-fpm"
    }
}

/// `php-fcgi@8.3.36` on Windows, `php-fpm@8.3.36` elsewhere: names a pool, and is the key of its ports.
pub(super) fn pool_id(os: Os, version: &str) -> String {
    format!("{}@{version}", pool_kind(os))
}

/// How many processes the pool `pool_id` has (or will have at its first start): the size it was last given on Windows,
/// where a pool is several `php-cgi` workers, [`DEFAULT_WORKERS`] before that, and 1 elsewhere. One rule for the page,
/// for starting and for stopping, so they can never disagree about which services belong to a pool.
pub(super) fn pool_size(ports: &PortAllocator, os: Os, pool_id: &str) -> Result<u16> {
    if os != Os::Windows {
        return Ok(1);
    }
    Ok(ports
        .assignment(pool_id)?
        .map_or(DEFAULT_WORKERS, |assigned| assigned.count))
}

/// The FastCGI ports of the pool of PHP `version`, **assigned if they were not yet**: the same assignment
/// [`php_definition`] makes (same id, same size), so a site written into the Caddyfile points at exactly the ports the
/// pool gets when it is started later.
///
/// # Errors
/// `NO_FREE_PORT` and I/O errors from the allocator.
pub fn pool_ports(ports: &PortAllocator, os: Os, version: &str) -> Result<Vec<u16>> {
    let pool = pool_id(os, version);
    let size = pool_size(ports, os, &pool)?;
    Ok(ports
        .assign(&pool, PortPurpose::PhpFastCgi { pool: size })?
        .ports())
}

/// The service ids behind a pool: the workers `-w1` and up on Windows, the pool itself elsewhere.
pub(super) fn pool_members(ports: &PortAllocator, os: Os, pool_id: &str) -> Result<Vec<String>> {
    if os != Os::Windows {
        return Ok(vec![pool_id.to_owned()]);
    }
    Ok((1..=usize::from(pool_size(ports, os, pool_id)?))
        .map(|n| worker_id(pool_id, n))
        .collect())
}

/// The service id of the `number`th `php-cgi` of a Windows pool (counted from 1): `php-fcgi@8.3.36-w1`.
pub(super) fn worker_id(pool_id: &str, number: usize) -> String {
    format!("{pool_id}-w{number}")
}

/// An installed PHP ready to run: what to start, and where PHP listens.
#[derive(Debug, Clone)]
pub struct PhpDefinition {
    /// The exact version, `8.3.36`.
    pub version: String,
    /// `php-fcgi@8.3.36` or `php-fpm@8.3.36`: names the pool and is the key of its ports in `config/services.json`.
    /// On Windows this is **not** a service id; the workers' are in `specs`.
    pub pool_id: String,
    /// Every FastCGI port of the pool, which are the upstreams of Caddy's `php_fastcgi`.
    pub ports: Vec<u16>,
    /// One spec per process: `workers` of them on Windows, one `php-fpm` elsewhere. Give each to `Supervisor::start`.
    pub specs: Vec<ServiceSpec>,
    /// macOS and Linux: the generated `php-fpm.conf`, rewritten on every definition. `None` on Windows.
    pub config_file: Option<PathBuf>,
}

/// Build the definition of one installed PHP, and write its generated `php-fpm.conf` (macOS and Linux).
///
/// - The ports come from the allocator and are kept for next time: `workers` consecutive ports on Windows (1 to
///   [`MAX_WORKERS`]; ignored elsewhere), one on macOS and Linux. A pool whose size changed gets new ports.
/// - Every process gets [`php_environment`] (`PHPRC` and `PHP_INI_SCAN_DIR`), so it reads the `php.ini` generated at
///   install and the user's `conf.d`.
/// - `os` is a parameter so both variants are tested on any machine. Pass `Os::current()`.
///
/// # Errors
/// `INTERNAL` for a runtime that is not PHP or a pool size that is 0 or over [`MAX_WORKERS`], `RUNTIME_BROKEN` when
/// the `php.ini` generated at install is gone, `NO_FREE_PORT` from the allocator, and I/O errors.
pub fn php_definition(
    home: &LunaHome,
    ports: &PortAllocator,
    runtime: &InstalledRuntime,
    os: Os,
    workers: u16,
) -> Result<PhpDefinition> {
    if runtime.runtime != "php" {
        return Err(LunaError::internal(format!(
            "{} is not PHP",
            runtime.runtime
        )));
    }
    let windows = os == Os::Windows;
    if windows && !(1..=MAX_WORKERS).contains(&workers) {
        return Err(LunaError::internal(format!(
            "a PHP pool has 1 to {MAX_WORKERS} workers, not {workers}"
        )));
    }
    let version = runtime.version.clone();
    let paths = php_config_paths(home, &version)?;
    if !paths.ini.is_file() {
        return Err(LunaError::new(
            ErrorCode::RuntimeBroken,
            format!(
                "the php.ini that lunaRun generated for PHP {version} is missing ({}): install this PHP again",
                paths.ini.display()
            ),
        ));
    }
    let env: Vec<(String, String)> = php_environment(home, &version)?
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_string_lossy().into_owned()))
        .collect();

    let pool_id = pool_id(os, &version);
    let count = if windows { workers } else { 1 };
    let assigned = ports.assign(&pool_id, PortPurpose::PhpFastCgi { pool: count })?;
    let pool_ports = assigned.ports();
    let ini = paths.ini.display().to_string();

    if windows {
        let specs = pool_ports
            .iter()
            .enumerate()
            .map(|(index, &port)| {
                let mut spec = ServiceSpec::new(
                    &worker_id(&pool_id, index + 1),
                    runtime.dir.join("php-cgi.exe"),
                    port,
                );
                spec.args = vec![
                    "-b".to_owned(),
                    format!("127.0.0.1:{port}"),
                    "-c".to_owned(),
                    ini.clone(),
                ];
                spec.env = env.clone();
                spec.env
                    .push(("PHP_FCGI_MAX_REQUESTS".to_owned(), MAX_REQUESTS.to_owned()));
                spec.cwd = Some(runtime.dir.clone());
                // Stateless: nothing to finish, so no polite request first.
                spec.stop = StopPlan {
                    method: StopMethod::Kill,
                    grace: Duration::from_secs(5),
                };
                spec.restart.recycle_on_exit = true;
                spec
            })
            .collect();
        return Ok(PhpDefinition {
            version,
            pool_id,
            ports: pool_ports,
            specs,
            config_file: None,
        });
    }

    let port = assigned.port;
    let config_file = paths.dir.join("php-fpm.conf");
    let error_log = home.logs_dir().join(format!("php-fpm-{version}.log"));
    std::fs::create_dir_all(home.logs_dir()).at_path(&home.logs_dir())?;
    let text = TEMPLATE
        .replace("{{error_log}}", &quoted(&error_log)?)
        .replace("{{port}}", &port.to_string());
    write_atomic(&config_file, text.as_bytes())?;

    let mut spec = ServiceSpec::new(&pool_id, runtime.dir.join("php-fpm"), port);
    spec.args = vec![
        "--nodaemonize".to_owned(),
        "--fpm-config".to_owned(),
        config_file.display().to_string(),
        "-c".to_owned(),
        ini,
    ];
    spec.env = env;
    spec.cwd = Some(runtime.dir.clone());
    // `SIGQUIT` lets it finish the requests it is serving (SERVICES section 2).
    spec.stop = StopPlan {
        method: StopMethod::Quit,
        grace: FPM_STOP_GRACE,
    };
    spec.log_files = vec![LogFile {
        source: "PHP-FPM log".to_owned(),
        path: error_log,
    }];
    Ok(PhpDefinition {
        version,
        pool_id,
        ports: pool_ports,
        specs: vec![spec],
        config_file: Some(config_file),
    })
}
