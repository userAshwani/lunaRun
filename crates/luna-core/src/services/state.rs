//! The life of one service as a table: which state follows which event (docs/SERVICES.md section 2).
//!
//! ```text
//!          Start               Ready
//! Stopped ───────► Starting ─────────► Running ──Recycle──► Starting
//!    ▲  Crashed/Error ─Start─►  │ StartFailed    │ Crash
//!    │                          ▼                ▼
//!    │                        Error           Crashed ──Start (auto-restart)──► Starting
//!    │                                                      Starting ──Crash──► Crashed
//!    └─ Stopped ◄──────── Stopping ◄──── Stop ──── Starting | Running
//!       Stop on Crashed or Error: straight to Stopped (nothing is running)
//! ```
//!
//! The table is a pure function so every legal move and every illegal one is tested without a process.
//! One departure from the diagram in SERVICES section 2: when the automatic restarts run out the service
//! **stays `Crashed`** (with `gave_up` set) instead of moving to `Error`, because FR-SV-06 says so ("then stay
//! in `crashed` and notify"). `Error` is for a start that never worked (it could not be launched, it exited
//! or timed out before it was ready), which restarting would not fix.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::LunaError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServiceState {
    /// Not running, and nothing wants it to.
    Stopped,
    /// The process was launched and is not ready yet.
    Starting,
    /// Ready and serving.
    Running,
    /// A stop was asked for; the process is being asked to leave, then forced.
    Stopping,
    /// The process died on its own while running (or an automatic restart failed).
    Crashed,
    /// A start that was asked for did not work. Not retried automatically.
    Error,
}

/// What can happen to a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Someone asked for it to run, or an automatic restart begins.
    Start,
    /// The ready check passed.
    Ready,
    /// The first start never became ready: not launched, exited early, or timed out.
    StartFailed,
    /// The process died without being asked to (also an automatic restart that fails before it is ready).
    Crash,
    /// The process exits by design (Windows `php-cgi` after its request limit) and is started again at once.
    Recycle,
    /// Someone asked for it to stop.
    Stop,
    /// The process is gone after a stop.
    Stopped,
}

impl ServiceState {
    /// The state after `event`, or `None` when `event` makes no sense here.
    pub fn next(self, event: Event) -> Option<Self> {
        use Event::{Crash, Ready, Recycle, Start, StartFailed, Stop, Stopped};
        use ServiceState as S;
        match (self, event) {
            (S::Stopped | S::Crashed | S::Error, Start) => Some(S::Starting),
            (S::Starting, Ready) => Some(S::Running),
            (S::Starting, StartFailed) => Some(S::Error),
            (S::Running | S::Starting, Crash) => Some(S::Crashed),
            (S::Running, Recycle) => Some(S::Starting),
            (S::Starting | S::Running, Stop) => Some(S::Stopping),
            (S::Crashed | S::Error, Stop) => Some(S::Stopped),
            (S::Stopping, Stopped) => Some(S::Stopped),
            _ => None,
        }
    }

    /// A process exists (or is being created or removed) for this state.
    pub fn has_process(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}

/// A snapshot of one service, as the UI shows it and as events carry it (FR-SV-04).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceStatus {
    /// For example `mysql@8.4` or `caddy`.
    pub id: String,
    pub state: ServiceState,
    /// Process id while there is a process.
    #[ts(type = "number | null")]
    pub pid: Option<u32>,
    /// Automatic restarts since the last time someone started the service.
    pub restarts: u32,
    /// How the last process ended, when it ended by itself.
    pub exit_code: Option<i32>,
    /// Why the last start failed (`SERVICE_START_FAILED`), or why the last process died.
    pub error: Option<LunaError>,
    /// The automatic restarts ran out: the service stays `Crashed` and the user should be told (FR-SV-06).
    pub gave_up: bool,
}

impl ServiceStatus {
    pub fn stopped(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            state: ServiceState::Stopped,
            pid: None,
            restarts: 0,
            exit_code: None,
            error: None,
            gave_up: false,
        }
    }

    /// Nothing will change by itself any more: it runs, it is stopped, a start failed, or the restarts ran out.
    /// A waiting caller (for "Start all") can stop waiting.
    pub fn is_settled(&self) -> bool {
        match self.state {
            ServiceState::Running | ServiceState::Stopped | ServiceState::Error => true,
            ServiceState::Crashed => self.gave_up,
            ServiceState::Starting | ServiceState::Stopping => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Event::{Crash, Ready, Recycle, Start, StartFailed, Stop, Stopped};
    use super::ServiceState::{self as S, Crashed, Error, Running, Starting, Stopping};
    use super::*;

    const STATES: [ServiceState; 6] = [S::Stopped, Starting, Running, Stopping, Crashed, Error];
    const EVENTS: [Event; 7] = [Start, Ready, StartFailed, Crash, Recycle, Stop, Stopped];

    /// Every legal move, written out. Anything not listed here must be refused.
    const LEGAL: [(ServiceState, Event, ServiceState); 13] = [
        (S::Stopped, Start, Starting),
        (Crashed, Start, Starting),
        (Error, Start, Starting),
        (Starting, Ready, Running),
        (Starting, StartFailed, Error),
        (Starting, Crash, Crashed),
        (Running, Crash, Crashed),
        (Running, Recycle, Starting),
        (Starting, Stop, Stopping),
        (Running, Stop, Stopping),
        (Crashed, Stop, S::Stopped),
        (Error, Stop, S::Stopped),
        (Stopping, Stopped, S::Stopped),
    ];

    #[test]
    fn every_legal_move_leads_where_the_table_says() {
        for (from, event, to) in LEGAL {
            assert_eq!(from.next(event), Some(to), "{from:?} + {event:?}");
        }
    }

    #[test]
    fn every_other_combination_is_refused() {
        let mut checked = 0;
        for from in STATES {
            for event in EVENTS {
                let legal = LEGAL.iter().any(|&(f, e, _)| f == from && e == event);
                if !legal {
                    assert_eq!(
                        from.next(event),
                        None,
                        "{from:?} + {event:?} must be refused"
                    );
                    checked += 1;
                }
            }
        }
        // 6 states x 7 events = 42 combinations, 13 of them legal.
        assert_eq!(checked, 42 - 13);
    }

    #[test]
    fn a_running_service_cannot_be_started_again_and_a_stopped_one_cannot_be_stopped_again() {
        assert_eq!(Running.next(Start), None);
        assert_eq!(Starting.next(Start), None);
        assert_eq!(Stopping.next(Start), None, "wait for the stop to finish");
        assert_eq!(S::Stopped.next(Stop), None);
        assert_eq!(Stopping.next(Stop), None);
    }

    #[test]
    fn only_states_with_a_process_say_so() {
        let with: Vec<_> = STATES.into_iter().filter(|s| s.has_process()).collect();
        assert_eq!(with, [Starting, Running, Stopping]);
    }

    #[test]
    fn a_status_is_settled_when_nothing_will_change_by_itself() {
        let mut status = ServiceStatus::stopped("caddy");
        for (state, gave_up, settled) in [
            (S::Stopped, false, true),
            (Starting, false, false),
            (Running, false, true),
            (Stopping, false, false),
            (Error, false, true),
            (Crashed, false, false), // a restart is coming
            (Crashed, true, true),   // the restarts ran out
        ] {
            status.state = state;
            status.gave_up = gave_up;
            assert_eq!(status.is_settled(), settled, "{state:?} gave_up={gave_up}");
        }
    }

    #[test]
    fn the_page_gets_camel_case_json() {
        let mut status = ServiceStatus::stopped("mysql@8.4");
        status.state = Crashed;
        status.pid = Some(4242);
        status.gave_up = true;
        status.exit_code = Some(1);
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["id"], "mysql@8.4");
        assert_eq!(value["state"], "crashed");
        assert_eq!(value["pid"], 4242);
        assert_eq!(value["exitCode"], 1);
        assert_eq!(value["gaveUp"], true);
        assert_eq!(value["restarts"], 0);
        assert!(value["error"].is_null());
    }
}
