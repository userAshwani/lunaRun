//! What lunaRun can run, for the Services page: one row per service, built from what is installed (D-031).
//!
//! **Read-only.** [`list_services`] writes nothing and assigns no port: the definitions
//! ([`database_definition`](super::database_definition), [`php_definition`](super::php_definition),
//! [`caddy_definition`](super::caddy_definition)) write config files and take ports, which is right when a service
//! is started and wrong every time a page is opened. A row therefore shows the ports a service **already has** (none
//! until it was first started) and asks the supervisor only for the state.
//!
//! - **Databases:** one row per engine and line (`mysql@8.4`). The patch versions of a line share one data folder, so
//!   the row runs from the **newest installed patch** of the line.
//! - **PHP:** one row per installed version, named by the pool (`php-fcgi@8.3.36`, `php-fpm@8.3.36`). On Windows the
//!   pool is several services (`-w1` and up, D-028) whose states are summed up in the row.
//! - **Caddy:** one row, from the newest installed Caddy.
//!
//! The same rules say which running service a runtime version is behind: [`running_from`] and [`usage_checker`] are
//! what stops an uninstall of a version that a running service was started from.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::databases::{Engine, data_dir_is_empty, data_dir_of};
use super::php::{DEFAULT_WORKERS, pool_id, worker_id};
use super::ports::PortAllocator;
use super::state::{ServiceState, ServiceStatus};
use super::supervisor::Supervisor;
use crate::diagnostics::Os;
use crate::error::Result;
use crate::paths::LunaHome;
use crate::registry::RuntimeVersion;
use crate::runtimes::{InstalledRuntime, RuntimeUsage, UsageChecker, list_installed};

/// What a service is, which decides where the page puts it and what it says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServiceKind {
    /// MySQL or MariaDB.
    Database,
    /// A PHP version's FastCGI pool.
    Php,
    /// Caddy.
    Web,
}

/// One line of the Services page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceRow {
    /// `mysql@8.4`, `php-fcgi@8.3.36`, `php-fpm@8.3.36` or `caddy`. For a Windows PHP pool this is the pool, not a
    /// service id: the services are in `members`.
    pub id: String,
    pub kind: ServiceKind,
    /// The runtime it runs from: `mysql`, `mariadb`, `php` or `caddy`.
    pub runtime: String,
    /// The installed version it runs from, exactly (`8.4.13`).
    pub version: String,
    /// `8.4` for a database, which belongs to its line; `None` for the others.
    pub line: Option<String>,
    /// The ports it has been given. Empty until it was started for the first time.
    pub ports: Vec<u16>,
    /// The state of the row: that of its only service, or the sum of a pool's (see [`aggregate_state`]).
    pub state: ServiceState,
    /// How many of its services run, and how many it has (1 and 1, or a Windows pool's workers).
    pub running: u32,
    pub total: u32,
    /// The services behind the row, with their own status (`killed`, `gaveUp`, the error of a failed start).
    pub members: Vec<ServiceStatus>,
    /// A database whose data folder does not exist yet: its first start sets it up, which takes a while.
    pub needs_setup: bool,
}

/// The state of a row from the states of its services. One service: its own. A pool: `Stopped` when none runs,
/// `Running` when all do, otherwise the most pressing thing (`Error`, `Crashed`, `Stopping`), and `Starting` for
/// what is on its way up (or a mix of running and stopped, which only exists while a pool is being started).
pub fn aggregate_state(members: &[ServiceStatus]) -> ServiceState {
    use ServiceState as S;
    let any = |state: S| members.iter().any(|m| m.state == state);
    let all = |state: S| members.iter().all(|m| m.state == state);
    if members.is_empty() || all(S::Stopped) {
        S::Stopped
    } else if all(S::Running) {
        S::Running
    } else if any(S::Error) {
        S::Error
    } else if any(S::Crashed) {
        S::Crashed
    } else if any(S::Stopping) {
        S::Stopping
    } else {
        S::Starting
    }
}

/// The newest of `runtimes` by version number (an unreadable version counts as the oldest).
fn newest<'a>(
    runtimes: impl Iterator<Item = &'a InstalledRuntime>,
) -> Option<&'a InstalledRuntime> {
    runtimes.max_by(|a, b| {
        RuntimeVersion::parse(&a.version)
            .cmp(&RuntimeVersion::parse(&b.version))
            .then_with(|| a.version.cmp(&b.version))
    })
}

/// The installed versions of one runtime.
fn of<'a>(
    installed: &'a [InstalledRuntime],
    runtime: &'a str,
) -> impl Iterator<Item = &'a InstalledRuntime> {
    installed.iter().filter(move |r| r.runtime == runtime)
}

fn line_of(version: &str) -> Option<String> {
    RuntimeVersion::parse(version).map(|v| v.major_minor())
}

/// Every service lunaRun can run from `installed`, in the order the page shows them: the web server, PHP (newest
/// first), then the databases. Reads the port assignments and asks the supervisor for states; writes nothing.
///
/// `os` is a parameter so that both variants (a Windows pool of `php-cgi` workers, one `php-fpm` elsewhere) are tested
/// on any machine. Pass `Os::current()`.
///
/// # Errors
/// I/O and state-file errors from reading the port assignments.
pub fn list_services(
    ports: &PortAllocator,
    supervisor: &Supervisor,
    home: &LunaHome,
    installed: &[InstalledRuntime],
    os: Os,
) -> Result<Vec<ServiceRow>> {
    let status_of = |id: &str| {
        supervisor
            .status(id)
            .unwrap_or_else(|| ServiceStatus::stopped(id))
    };
    let row = |id: String,
               kind: ServiceKind,
               from: &InstalledRuntime,
               line: Option<String>,
               member_ids: Vec<String>,
               needs_setup: bool|
     -> Result<ServiceRow> {
        let members: Vec<ServiceStatus> = member_ids.iter().map(|m| status_of(m)).collect();
        let count = |state: ServiceState| {
            u32::try_from(members.iter().filter(|m| m.state == state).count()).unwrap_or(u32::MAX)
        };
        Ok(ServiceRow {
            ports: ports
                .assignment(&id)?
                .map(|assigned| assigned.ports())
                .unwrap_or_default(),
            id,
            kind,
            runtime: from.runtime.clone(),
            version: from.version.clone(),
            line,
            state: aggregate_state(&members),
            running: count(ServiceState::Running),
            total: u32::try_from(members.len()).unwrap_or(u32::MAX),
            members,
            needs_setup,
        })
    };
    let mut rows = Vec::new();

    if let Some(caddy) = newest(of(installed, "caddy")) {
        rows.push(row(
            "caddy".to_owned(),
            ServiceKind::Web,
            caddy,
            None,
            vec!["caddy".to_owned()],
            false,
        )?);
    }

    let mut php: Vec<&InstalledRuntime> = of(installed, "php").collect();
    php.sort_by(|a, b| {
        RuntimeVersion::parse(&b.version)
            .cmp(&RuntimeVersion::parse(&a.version))
            .then_with(|| b.version.cmp(&a.version))
    });
    for runtime in php {
        let pool = pool_id(os, &runtime.version);
        let workers = if os == Os::Windows {
            // The size the pool was last given, or the default it will get at its first start.
            ports
                .assignment(&pool)?
                .map_or(DEFAULT_WORKERS, |assigned| assigned.count)
        } else {
            1
        };
        let members = if os == Os::Windows {
            (1..=usize::from(workers))
                .map(|n| worker_id(&pool, n))
                .collect()
        } else {
            vec![pool.clone()]
        };
        rows.push(row(pool, ServiceKind::Php, runtime, None, members, false)?);
    }

    // Databases: the newest patch of each line, engines in alphabetical order, lines newest first.
    for engine in [Engine::Mariadb, Engine::Mysql] {
        let mut lines: Vec<String> = of(installed, engine.name())
            .filter_map(|r| line_of(&r.version))
            .collect();
        lines.sort_by(|a, b| RuntimeVersion::parse(b).cmp(&RuntimeVersion::parse(a)));
        lines.dedup();
        for line in lines {
            let Some(runtime) = newest(
                of(installed, engine.name())
                    .filter(|r| line_of(&r.version).as_deref() == Some(line.as_str())),
            ) else {
                continue;
            };
            let id = format!("{}@{line}", engine.name());
            let needs_setup = data_dir_is_empty(&data_dir_of(home, engine, &line));
            rows.push(row(
                id.clone(),
                ServiceKind::Database,
                runtime,
                Some(line),
                vec![id],
                needs_setup,
            )?);
        }
    }
    Ok(rows)
}

/// The ids of the services running (or starting, or stopping) from `runtime` `version`: what makes an uninstall of
/// that version refuse with `RUNTIME_IN_USE`. A database line runs from the newest patch of the line, so an older
/// patch of the same line is not in use; a PHP version has a pool of its own.
///
/// # Errors
/// I/O and state-file errors from reading what is installed and the port assignments.
pub fn running_from(
    ports: &PortAllocator,
    supervisor: &Supervisor,
    home: &LunaHome,
    os: Os,
    runtime: &str,
    version: &str,
) -> Result<Vec<String>> {
    let installed = list_installed(home)?;
    Ok(list_services(ports, supervisor, home, &installed, os)?
        .into_iter()
        .filter(|row| row.runtime == runtime && row.version == version)
        .flat_map(|row| row.members)
        .filter(|member| member.state.has_process())
        .map(|member| member.id)
        .collect())
}

/// The answer the uninstaller asks for ([`Uninstaller::with_usage`](crate::runtimes::Uninstaller::with_usage)): which
/// running services were started from a version. Bound projects stay empty until the project store exists (M1.7).
pub fn usage_checker(
    ports: PortAllocator,
    supervisor: Supervisor,
    home: LunaHome,
    os: Os,
) -> UsageChecker {
    Arc::new(move |runtime, version| {
        Ok(RuntimeUsage {
            running_services: running_from(&ports, &supervisor, &home, os, runtime, version)?,
            bound_projects: Vec::new(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, state: ServiceState) -> ServiceStatus {
        ServiceStatus {
            state,
            ..ServiceStatus::stopped(id)
        }
    }

    fn sum(states: &[ServiceState]) -> ServiceState {
        let members: Vec<ServiceStatus> = states
            .iter()
            .enumerate()
            .map(|(n, state)| member(&format!("w{n}"), *state))
            .collect();
        aggregate_state(&members)
    }

    #[test]
    fn a_single_service_is_in_its_own_state() {
        use ServiceState as S;
        for state in [
            S::Stopped,
            S::Starting,
            S::Running,
            S::Stopping,
            S::Crashed,
            S::Error,
        ] {
            assert_eq!(sum(&[state]), state, "{state:?}");
        }
    }

    #[test]
    fn a_pool_is_stopped_or_running_only_when_all_of_it_is() {
        use ServiceState as S;
        assert_eq!(sum(&[]), S::Stopped, "nothing at all");
        assert_eq!(sum(&[S::Stopped, S::Stopped, S::Stopped]), S::Stopped);
        assert_eq!(sum(&[S::Running, S::Running, S::Running]), S::Running);
        // On its way up, or a mix while it is being started.
        assert_eq!(sum(&[S::Running, S::Starting, S::Starting]), S::Starting);
        assert_eq!(sum(&[S::Running, S::Stopped]), S::Starting);
    }

    #[test]
    fn the_most_pressing_state_of_a_pool_wins() {
        use ServiceState as S;
        assert_eq!(sum(&[S::Running, S::Error, S::Stopping]), S::Error);
        assert_eq!(sum(&[S::Running, S::Crashed, S::Stopping]), S::Crashed);
        assert_eq!(sum(&[S::Running, S::Stopping, S::Starting]), S::Stopping);
        assert_eq!(sum(&[S::Crashed, S::Stopped, S::Stopped]), S::Crashed);
    }
}
