//! The Caddy service: the web server in front of every site (FR-SV-02, FR-ST-03; docs/SERVICES.md sections 1, 2 and 4;
//! D-028).
//!
//! [`caddy_definition`] turns the installed Caddy and the **list of sites** into a ready [`ServiceSpec`] and the
//! generated Caddyfile. The Caddyfile is **owned by lunaRun and rewritten every time**; when the sites change, the
//! caller builds a new definition, [`validate`](CaddyDefinition::validate)s it and, if Caddy is running,
//! [`reload`](CaddyDefinition::reload)s it: no restart, so the other sites stay up (FR-ST-03). A file that
//! fails validation never reaches the running Caddy.
//!
//! A site's folder is the user's own project, so it only appears as **text in the Caddyfile**: lunaRun never writes
//! there (golden rule 3).

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::databases::{plain, quoted};
use super::logs::last_lines;
use super::ports::{PortAllocator, PortPurpose};
use super::process::{RunEnd, run_logged};
use super::spec::{HelperCommand, LogFile, ServiceSpec, StopMethod, StopPlan};
use crate::diagnostics::Os;
use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::paths::{LunaHome, component};
use crate::runtimes::InstalledRuntime;
use crate::state::write_atomic;

/// `caddy stop` asks the admin API to shut down; after this Caddy is killed (SERVICES section 2).
const STOP_GRACE: Duration = Duration::from_secs(10);
/// How many lines of what Caddy said go into an error.
const TAIL_LINES: usize = 15;
/// A site name names its access log file and its log tab.
const MAX_SITE_NAME: usize = 40;

/// One site: a folder served on a port, with or without PHP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// Letters, digits and `. _ -`; names the access log (`logs/sites/<name>.access.log`) and its tab.
    pub name: String,
    /// The site is `http://localhost:<port>`. The caller gets it from the allocator (`PortPurpose::Site`).
    pub port: u16,
    /// The folder that is served: `public` of a Laravel project. Only written into the Caddyfile.
    pub root: PathBuf,
    /// The FastCGI ports of the PHP pool this site uses (`PhpDefinition::ports`). Empty: static files only.
    pub php_ports: Vec<u16>,
}

/// Caddy ready to run, and the commands that change it while it runs.
#[derive(Debug, Clone)]
pub struct CaddyDefinition {
    /// Give this to `Supervisor::start`.
    pub spec: ServiceSpec,
    pub admin_port: u16,
    /// The generated `config/caddy/Caddyfile`.
    pub config_file: PathBuf,
    pub sites: Vec<Site>,
    /// `caddy validate`: public so a test can stand in for it.
    pub validate_command: HelperCommand,
    /// `caddy reload`: public so a test can stand in for it.
    pub reload_command: HelperCommand,
    /// What `validate` and `reload` printed, kept for the case that one fails.
    pub check_log: PathBuf,
    home: LunaHome,
}

/// A path as the Caddyfile wants it: forward slashes, in quotes. Caddy also expands `{placeholders}` inside quotes,
/// so a folder with braces in its name cannot be written down faithfully and is refused.
fn caddy_path(path: &Path) -> Result<String> {
    if path.to_string_lossy().contains(['{', '}']) {
        return Err(LunaError::new(
            ErrorCode::InvalidPath,
            format!(
                "{} cannot be served: Caddy reads braces in a path as a placeholder",
                path.display()
            ),
        )
        .with_details(json!({ "path": path.display().to_string() })));
    }
    quoted(path)
}

fn check_sites(sites: &[Site], admin_port: u16) -> Result<()> {
    for (index, site) in sites.iter().enumerate() {
        component(&site.name)?;
        if site.name.len() > MAX_SITE_NAME {
            return Err(LunaError::internal(format!(
                "the site name {:?} is longer than {MAX_SITE_NAME} characters",
                site.name
            )));
        }
        if site.port == 0 || site.port == admin_port {
            return Err(LunaError::internal(format!(
                "site {} cannot use port {}",
                site.name, site.port
            )));
        }
        if site.php_ports.contains(&0) {
            return Err(LunaError::internal(format!(
                "site {} has a PHP port 0",
                site.name
            )));
        }
        if sites[..index]
            .iter()
            .any(|other| other.name == site.name || other.port == site.port)
        {
            return Err(LunaError::internal(format!(
                "two sites share the name or the port of {} ({})",
                site.name, site.port
            )));
        }
    }
    Ok(())
}

fn access_log(home: &LunaHome, site: &Site) -> PathBuf {
    home.logs_dir()
        .join("sites")
        .join(format!("{}.access.log", site.name))
}

/// The Caddyfile of SERVICES section 4, for these sites.
///
/// - Tabs, as `caddy fmt` writes them (Caddy warns about a file that is not formatted).
/// - `bind 127.0.0.1`: a development site is not offered to the network, and nothing here ever asks the
///   firewall. Both `localhost` and `127.0.0.1` name the site: a block for one host name only answers requests
///   that say that name.
/// - `php_fastcgi` already sends what is not a file to `index.php` (Laravel's front controller).
fn render_caddyfile(home: &LunaHome, admin_port: u16, sites: &[Site]) -> Result<String> {
    let mut text = String::from(
        "# Generated by lunaRun. Changes made here are overwritten whenever the sites change.\n",
    );
    text.push_str(&format!(
        "{{\n\tadmin 127.0.0.1:{admin_port}\n\tauto_https off\n\tpersist_config off\n}}\n"
    ));
    for site in sites {
        let port = site.port;
        text.push_str(&format!(
            "\nhttp://localhost:{port}, http://127.0.0.1:{port} {{\n"
        ));
        text.push_str("\tbind 127.0.0.1\n");
        text.push_str(&format!("\troot * {}\n", caddy_path(&site.root)?));
        text.push_str("\tencode gzip\n");
        if !site.php_ports.is_empty() {
            let upstreams: Vec<String> = site
                .php_ports
                .iter()
                .map(|p| format!("127.0.0.1:{p}"))
                .collect();
            text.push_str(&format!("\tphp_fastcgi {}\n", upstreams.join(" ")));
        }
        text.push_str("\tfile_server\n");
        text.push_str(&format!(
            "\tlog {{\n\t\toutput file {}\n\t}}\n}}\n",
            caddy_path(&access_log(home, site))?
        ));
    }
    Ok(text)
}

/// Build the definition of Caddy for `sites`, and write its Caddyfile.
///
/// - The admin port comes from the allocator (2119, which is not Caddy's default 2019 so a Caddy of the user's own is
///   left alone) and is kept for next time.
/// - The service listens on the admin port and every site's port; the supervisor checks them all before it starts.
///   Ready means the first site answers (the admin API is up a moment before the sites), or the admin API when
///   there are no sites.
/// - Caddy keeps its own state under `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, which would otherwise be your user
///   profile (`%AppData%`, `~/.local/share`): both point inside `LUNA_HOME`.
/// - `os` is a parameter so both variants are tested on any machine. Pass `Os::current()`.
///
/// # Errors
/// `INTERNAL` for a runtime that is not Caddy or sites that clash (a name or port used twice, the admin port, port
/// 0), `INVALID_PATH` for a path that cannot be written into a Caddyfile, `PORT_IN_USE` for the admin port when
/// something else holds it, and I/O errors.
pub fn caddy_definition(
    home: &LunaHome,
    ports: &PortAllocator,
    runtime: &InstalledRuntime,
    sites: &[Site],
    os: Os,
) -> Result<CaddyDefinition> {
    if runtime.runtime != "caddy" {
        return Err(LunaError::internal(format!(
            "{} is not Caddy",
            runtime.runtime
        )));
    }
    let admin_port = ports.assign("caddy", PortPurpose::CaddyAdmin)?.port;
    check_sites(sites, admin_port)?;

    let config_dir = home.config_dir().join("caddy");
    let config_file = config_dir.join("Caddyfile");
    fs::create_dir_all(&config_dir).at_path(&config_dir)?;
    fs::create_dir_all(home.logs_dir().join("sites")).at_path(&home.logs_dir())?;
    write_atomic(
        &config_file,
        render_caddyfile(home, admin_port, sites)?.as_bytes(),
    )?;

    let program = runtime.dir.join(if os == Os::Windows {
        "caddy.exe"
    } else {
        "caddy"
    });
    let admin = format!("127.0.0.1:{admin_port}");
    let file = plain(&config_file)?;
    let env = vec![
        (
            "XDG_DATA_HOME".to_owned(),
            plain(&home.data_dir().join("caddy"))?,
        ),
        (
            "XDG_CONFIG_HOME".to_owned(),
            plain(&config_dir.join("state"))?,
        ),
    ];
    let helper = |args: &[&str]| HelperCommand {
        program: program.clone(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        env: env.clone(),
    };

    let ready_port = sites.first().map_or(admin_port, |site| site.port);
    let mut spec = ServiceSpec::new("caddy", program.clone(), ready_port);
    spec.args = ["run", "--config", &file, "--adapter", "caddyfile"]
        .map(str::to_owned)
        .to_vec();
    spec.env = env.clone();
    spec.cwd = Some(config_dir);
    spec.ports = std::iter::once(admin_port)
        .chain(sites.iter().map(|site| site.port))
        .collect();
    spec.stop = StopPlan {
        method: StopMethod::Command(helper(&["stop", "--address", &admin])),
        grace: STOP_GRACE,
    };
    spec.log_files = sites
        .iter()
        .map(|site| LogFile {
            source: format!("{} access log", site.name),
            path: access_log(home, site),
        })
        .collect();

    Ok(CaddyDefinition {
        spec,
        admin_port,
        validate_command: helper(&["validate", "--config", &file, "--adapter", "caddyfile"]),
        reload_command: helper(&[
            "reload",
            "--config",
            &file,
            "--adapter",
            "caddyfile",
            "--address",
            &admin,
        ]),
        check_log: home.logs_dir().join("caddy.check.log"),
        config_file,
        sites: sites.to_vec(),
        home: home.clone(),
    })
}

impl CaddyDefinition {
    /// Ask Caddy whether it accepts the generated Caddyfile (`caddy validate`). Nothing running is touched. Call it
    /// before [`reload`](Self::reload) or the first start.
    ///
    /// # Errors
    /// `CONFIG_INVALID` with what Caddy said (`details.tail`, also in `check_log`), `CANCELLED`, and I/O errors.
    pub async fn validate(&self, limit: Duration, cancel: &CancellationToken) -> Result<()> {
        self.run_check(
            &self.validate_command,
            "validating",
            ErrorCode::ConfigInvalid,
            limit,
            cancel,
        )
        .await
    }

    /// Make the **running** Caddy load the generated Caddyfile (`caddy reload`): sites are added, changed or removed
    /// without a restart. A reload that fails leaves Caddy on its old config (Caddy's own rule), so the other sites
    /// stay up. Validate first.
    ///
    /// # Errors
    /// `RELOAD_FAILED` with what Caddy said (also when Caddy is not running), `CANCELLED`, and I/O errors.
    pub async fn reload(&self, limit: Duration, cancel: &CancellationToken) -> Result<()> {
        self.run_check(
            &self.reload_command,
            "reloading",
            ErrorCode::ReloadFailed,
            limit,
            cancel,
        )
        .await
    }

    async fn run_check(
        &self,
        helper: &HelperCommand,
        doing: &str,
        failure: ErrorCode,
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Result<()> {
        fs::create_dir_all(self.home.logs_dir()).at_path(&self.home.logs_dir())?;
        let log = File::create(&self.check_log).at_path(&self.check_log)?;
        let outcome = run_logged(helper, &log, limit, cancel).await;
        let why = match &outcome {
            Ok(RunEnd::Exited(Some(0))) => return Ok(()),
            Ok(RunEnd::Exited(Some(code))) => format!("Caddy answered with exit code {code}"),
            Ok(RunEnd::Exited(None)) => "Caddy was ended before it answered".to_owned(),
            Ok(RunEnd::TimedOut) => format!("Caddy did not answer within {} s", limit.as_secs()),
            Ok(RunEnd::Cancelled) => {
                return Err(LunaError::new(
                    ErrorCode::Cancelled,
                    format!("{doing} the Caddy configuration was cancelled"),
                ));
            }
            Err(err) => format!("Caddy could not be started: {err}"),
        };
        Err(
            LunaError::new(failure, format!("{doing} the Caddyfile: {why}")).with_details(json!({
                "service": "caddy",
                "file": self.config_file.display().to_string(),
                "log": self.check_log.display().to_string(),
                "tail": last_lines(&self.check_log, TAIL_LINES, 500),
            })),
        )
    }
}
