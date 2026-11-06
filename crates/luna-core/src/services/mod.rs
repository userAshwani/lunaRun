//! Services: the processes lunaRun keeps running (PHP, MySQL, MariaDB, Caddy) and the supervisor that owns them
//! (FR-SV-01, FR-SV-02, FR-SV-04, FR-SV-06; docs/SERVICES.md section 2).
//!
//! - [`ServiceState`] / [`Event`]: the state machine as a pure table.
//! - [`RestartPolicy`] / [`RestartTracker`]: when a crashed service is restarted, and when it is given up on.
//! - [`ServiceSpec`]: what to run, how to know it is ready, how to stop it. Validated before anything starts:
//!   programs only run from inside `LUNA_HOME`.
//! - [`Supervisor`]: starts, watches, restarts and stops services, and reports every state change to a
//!   [`ServiceEventSink`].
//!
//! - [`Containment`] / [`OsContainment`] and the PID files: no process outlives lunaRun unnoticed (FR-SV-07).
//!   Services left running by a crash are found again ([`Supervisor::scan_leftovers`]) and adopted or stopped.
//!
//! - [`LogConfig`], [`LogLine`], [`LogEventSink`]: what a service prints, kept in rotating files and a ring of the last
//!   lines, tailed live and sent in throttled batches (FR-SV-05, D-025).
//!
//! - [`PortAllocator`], [`check_ports`], [`port_owners`]: stable ports from the ranges of SERVICES section 3, the
//!   check before a start, and who holds a port that is taken (FR-SV-03).
//!
//! - [`database_definition`]: `mysql@<line>` and `mariadb@<line>` from an installed runtime (generated config, first-run
//!   setup, clean shutdown, error log tab).
//!
//! The PHP and Caddy service definitions are the next M1.4 task.

mod caddy;
mod containment;
mod databases;
mod gate;
mod logs;
mod os;
mod php;
mod pidfile;
mod port_owner;
mod ports;
mod process;
mod restart;
mod shutdown;
mod spec;
mod state;
mod supervisor;

pub use caddy::{CaddyDefinition, Site, caddy_definition};
pub use containment::{Containment, NoContainment, OsContainment};
pub use databases::{DatabaseDefinition, Engine, database_definition};
pub use logs::{LogBatch, LogConfig, LogEventSink, LogLine, OUTPUT};
pub use os::{process_alive, process_exe, same_program};
pub use php::{DEFAULT_WORKERS, MAX_WORKERS, PhpDefinition, php_definition};
pub use port_owner::{PortOwner, port_owners};
pub use ports::{PortAllocator, PortAssignment, PortPurpose, PortRanges, check_ports};
pub use restart::{Decision, RestartPolicy, RestartTracker};
pub use shutdown::{
    DEFAULT_GROUP_LIMIT, ShutdownFailure, ShutdownGroup, ShutdownProgress, ShutdownReport, group_of,
};
pub use spec::{HelperCommand, LogFile, ReadyCheck, ServiceSpec, StopMethod, StopPlan};
pub use state::{Event, ServiceState, ServiceStatus};
pub use supervisor::{Leftover, ServiceEventSink, Supervisor};
