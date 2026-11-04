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
//! Log capture, ports and the concrete service definitions are the later M1.4 tasks.

mod containment;
mod os;
mod pidfile;
mod process;
mod restart;
mod spec;
mod state;
mod supervisor;

pub use containment::{Containment, NoContainment, OsContainment};
pub use os::{process_alive, process_exe, same_program};
pub use restart::{Decision, RestartPolicy, RestartTracker};
pub use spec::{HelperCommand, ReadyCheck, ServiceSpec, StopMethod, StopPlan};
pub use state::{Event, ServiceState, ServiceStatus};
pub use supervisor::{Leftover, ServiceEventSink, Supervisor};
