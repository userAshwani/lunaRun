//! Ports for services (FR-SV-03, docs/SERVICES.md section 3).
//!
//! - [`PortAllocator`] hands out ports from the ranges set aside for each kind of service and **remembers them**
//!   in `LUNA_HOME/config/services.json`, so `mysql@8.4` is on the same port after a restart, and a port that was
//!   given to one service is never handed to another.
//! - [`check_ports`] is what a start does first: is `127.0.0.1:<port>` really free? If not it says
//!   `PORT_IN_USE`, with the program that holds the port when the OS tells (so the UI can say "XAMPP MySQL is
//!   running from C:\xampp\mysql\bin\mysqld.exe" and offer another port).
//!
//! A port is **taken** if a test bind fails *or* the OS lists a listener on it that would clash (see `port_owner.rs`:
//! on Windows and macOS the bind alone misses a program that listens on the wildcard address).

use std::collections::{BTreeMap, HashSet};
use std::net::{Ipv4Addr, TcpListener};
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

use super::port_owner::{self, PortOwner};
use super::spec::check_id;
use crate::error::{ErrorCode, LunaError, Result};
use crate::paths::LunaHome;
use crate::state::{self, Persisted};

/// The ranges of SERVICES section 3. Public so a test can use small ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRanges {
    /// One per project: `http://localhost:<port>`.
    pub site: RangeInclusive<u16>,
    /// Per PHP pool; Windows `php-cgi` takes several consecutive ports.
    pub php_fastcgi: RangeInclusive<u16>,
    /// MySQL and MariaDB: the first line gets 3306 if it is free, the next ones 3307 and up.
    pub database: RangeInclusive<u16>,
    /// Caddy's admin API. Not Caddy's default 2019, which a user's own Caddy may use. One port only.
    pub caddy_admin: RangeInclusive<u16>,
}

impl Default for PortRanges {
    fn default() -> Self {
        Self {
            site: 8100..=8999,
            php_fastcgi: 9100..=9999,
            database: 3306..=3399,
            caddy_admin: 2119..=2119,
        }
    }
}

/// What a port is for, which decides the range it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortPurpose {
    Site,
    /// `pool` consecutive ports (the Windows `php-cgi` pool; one for `php-fpm`).
    PhpFastCgi {
        pool: u16,
    },
    Database,
    CaddyAdmin,
}

/// The ports a service was given: `count` consecutive ports starting at `port`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortAssignment {
    pub port: u16,
    pub count: u16,
}

impl PortAssignment {
    pub fn ports(&self) -> Vec<u16> {
        (self.port..self.port.saturating_add(self.count)).collect()
    }
}

/// `config/services.json`. Only the ports for now; the service definitions will add to it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ServicesConfig {
    #[serde(default)]
    ports: BTreeMap<String, PortAssignment>,
}

impl Persisted for ServicesConfig {
    const SCHEMA_VERSION: u32 = 1;
}

// ---------------------------------------------------------------------------------------------
// Is a port free?
// ---------------------------------------------------------------------------------------------

/// Is `127.0.0.1:<port>` free to listen on? `Ok` if yes; `PORT_IN_USE` if not, naming the process that holds it
/// when the OS says. Call it right before a service starts.
pub fn check_ports(ports: &[u16]) -> Result<()> {
    for &port in ports {
        // The listener of a successful test bind must be gone before the system is asked who listens: it would be
        // listed, and every port would be reported as held by this very process.
        let bind_error = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).err();
        let listeners = port_owner::conflicting_listeners(port);
        if bind_error.is_some() || !listeners.is_empty() {
            let owner = port_owner::owners_of(listeners).into_iter().next();
            let reason = match bind_error {
                Some(error) => error.to_string(),
                None => "the system lists a program listening on it".to_owned(),
            };
            return Err(port_in_use(port, owner.as_ref(), Some(reason)));
        }
    }
    Ok(())
}

/// `reason` says why the port cannot be used when no program can be named (another user's, on Linux and macOS): the
/// error of the test bind, or what the system lists. "Address already in use" and "Permission denied" (a port range
/// Windows keeps for itself) are different problems, and without the reason both read the same.
fn port_in_use(port: u16, owner: Option<&PortOwner>, reason: Option<String>) -> LunaError {
    let mut details = Map::new();
    details.insert("port".into(), json!(port));
    let message = match owner {
        Some(owner) => {
            details.insert("pid".into(), json!(owner.pid));
            if let Some(name) = &owner.process_name {
                details.insert("processName".into(), json!(name));
            }
            if let Some(path) = &owner.process_path {
                details.insert("processPath".into(), json!(path));
            }
            match (&owner.process_name, &owner.process_path) {
                (Some(name), Some(path)) => {
                    format!("port {port} is already in use by {name} ({path})")
                }
                (Some(name), None) => format!("port {port} is already in use by {name}"),
                _ => format!("port {port} is already in use by process {}", owner.pid),
            }
        }
        None => match reason {
            Some(reason) => {
                details.insert("reason".into(), json!(reason));
                format!("port {port} is already in use ({reason})")
            }
            None => format!("port {port} is already in use"),
        },
    };
    LunaError::new(ErrorCode::PortInUse, message).with_details(Value::Object(details))
}

// ---------------------------------------------------------------------------------------------
// The allocator
// ---------------------------------------------------------------------------------------------

/// Hands out ports and remembers them. Cheap to clone; clones share one lock, so two threads never hand out the same
/// port.
#[derive(Clone)]
pub struct PortAllocator {
    home: LunaHome,
    ranges: PortRanges,
    lock: Arc<Mutex<()>>,
}

impl PortAllocator {
    pub fn new(home: LunaHome) -> Self {
        Self {
            home,
            ranges: PortRanges::default(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Other ranges than the ones in SERVICES section 3 (for tests).
    #[must_use]
    pub fn with_ranges(mut self, ranges: PortRanges) -> Self {
        self.ranges = ranges;
        self
    }

    fn file(&self) -> std::path::PathBuf {
        self.home.config_dir().join("services.json")
    }

    fn load(&self) -> Result<ServicesConfig> {
        Ok(state::load::<ServicesConfig>(&self.file())?.value)
    }

    fn save(&self, config: &ServicesConfig) -> Result<()> {
        state::save(&self.file(), config)
    }

    /// The ports of a service: what it was given before (the same ones, whatever is on them now: whether they are
    /// free is [`check_ports`]'s question, asked when it starts), or, the first time, the first free ports of the
    /// range for its purpose that no other service has. The choice is saved before this returns.
    pub fn assign(&self, id: &str, purpose: PortPurpose) -> Result<PortAssignment> {
        check_id(id)?;
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut config = self.load()?;
        let count = self.count_for(purpose)?;
        if let Some(existing) = config.ports.get(id)
            && existing.count == count
            && self.range_for(purpose).contains(&existing.port)
        {
            return Ok(*existing);
        }
        let chosen = self.choose(&config, id, purpose, None)?;
        config.ports.insert(id.to_owned(), chosen);
        self.save(&config)?;
        Ok(chosen)
    }

    /// "Use another port": give the service the next free ports of its range, different from the ones it has (the
    /// ones it has are taken, or the user wants to move). The old ports are released.
    pub fn reassign(&self, id: &str, purpose: PortPurpose) -> Result<PortAssignment> {
        check_id(id)?;
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut config = self.load()?;
        let current = config.ports.get(id).copied();
        let chosen = self.choose(&config, id, purpose, current)?;
        config.ports.insert(id.to_owned(), chosen);
        self.save(&config)?;
        Ok(chosen)
    }

    /// The service is gone: its ports can be handed to another. Nothing happens for an id that has none.
    pub fn release(&self, id: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut config = self.load()?;
        if config.ports.remove(id).is_some() {
            self.save(&config)?;
        }
        Ok(())
    }

    /// What a service was given, if anything.
    pub fn assignment(&self, id: &str) -> Result<Option<PortAssignment>> {
        Ok(self.load()?.ports.get(id).copied())
    }

    /// Every assignment, by service id.
    pub fn all(&self) -> Result<BTreeMap<String, PortAssignment>> {
        Ok(self.load()?.ports)
    }

    fn range_for(&self, purpose: PortPurpose) -> &RangeInclusive<u16> {
        match purpose {
            PortPurpose::Site => &self.ranges.site,
            PortPurpose::PhpFastCgi { .. } => &self.ranges.php_fastcgi,
            PortPurpose::Database => &self.ranges.database,
            PortPurpose::CaddyAdmin => &self.ranges.caddy_admin,
        }
    }

    fn count_for(&self, purpose: PortPurpose) -> Result<u16> {
        match purpose {
            PortPurpose::PhpFastCgi { pool: 0 } => {
                Err(LunaError::internal("a PHP pool needs at least one port"))
            }
            PortPurpose::PhpFastCgi { pool } => Ok(pool),
            _ => Ok(1),
        }
    }

    /// The first run of `count` consecutive ports in the range that is free now, is not another service's, and is not
    /// the one in `avoid`. A range with a single port (Caddy's admin port) that is taken is `PORT_IN_USE`, because
    /// there is no other port to offer; any other full range is `NO_FREE_PORT`.
    fn choose(
        &self,
        config: &ServicesConfig,
        id: &str,
        purpose: PortPurpose,
        avoid: Option<PortAssignment>,
    ) -> Result<PortAssignment> {
        let count = self.count_for(purpose)?;
        let range = self.range_for(purpose);
        let (first, last) = (*range.start(), *range.end());

        // What other services hold, and what the OS says is taken: one call each, not one per candidate.
        let assigned: HashSet<u16> = config
            .ports
            .iter()
            .filter(|(other, _)| other.as_str() != id)
            .flat_map(|(_, assignment)| assignment.ports())
            .collect();
        let listening = port_owner::listening_ports();
        let avoided: HashSet<u16> = avoid
            .map(|a| a.ports().into_iter().collect())
            .unwrap_or_default();

        let free = |port: u16| {
            !assigned.contains(&port)
                && !avoided.contains(&port)
                && !listening.contains(&port)
                && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
        };

        let mut start = u32::from(first);
        while start + u32::from(count) - 1 <= u32::from(last) {
            let window = start..start + u32::from(count);
            // `try_from` cannot fail: the window is inside a range of `u16`.
            if window.clone().all(|p| u16::try_from(p).is_ok_and(free)) {
                let port =
                    u16::try_from(start).map_err(|_| LunaError::internal("port out of range"))?;
                return Ok(PortAssignment { port, count });
            }
            start += 1;
        }

        if first == last {
            // The only port there is, and it is not free: say who has it, so the user can deal with it. If the
            // system finds nothing on it, another service of ours was given it.
            return Err(check_ports(&[first]).err().unwrap_or_else(|| {
                port_in_use(first, None, Some("another service has it".to_owned()))
            }));
        }
        Err(LunaError::new(
            ErrorCode::NoFreePort,
            format!("no free port in {first}-{last} for {id}"),
        )
        .with_details(json!({ "service": id, "first": first, "last": last, "count": count })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `check_ports` binds the port to test it. That listener must be gone before the system is asked who listens, or
    /// every free port is reported as held by this very process (it once was, and the tests hung looking for a port).
    #[test]
    fn a_free_port_is_not_reported_as_held_by_this_process() {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        check_ports(&[port]).unwrap();

        // And a port this process really holds is found, and is named.
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let held_port = held.local_addr().unwrap().port();
        let error = check_ports(&[held_port]).unwrap_err();
        assert_eq!(error.code, ErrorCode::PortInUse);
        assert_eq!(error.details.unwrap()["pid"], std::process::id());
    }

    #[test]
    fn a_port_with_no_known_owner_says_why_it_cannot_be_used() {
        let refused = std::io::Error::from(std::io::ErrorKind::PermissionDenied).to_string();
        let error = port_in_use(8100, None, Some(refused.clone()));
        assert_eq!(error.code, ErrorCode::PortInUse);
        assert!(
            error.message.starts_with("port 8100 is already in use ("),
            "{}",
            error.message
        );
        let details = error.details.unwrap();
        assert_eq!(details["port"], 8100);
        assert_eq!(details["reason"], refused);

        // Nothing is known: the plain message, as before.
        let error = port_in_use(8100, None, None);
        assert_eq!(error.message, "port 8100 is already in use");
    }
}
