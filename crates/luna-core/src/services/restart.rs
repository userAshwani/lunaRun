//! When a service dies on its own: restart it after a growing pause, and give up after too many in a short
//! time (FR-SV-06, docs/SERVICES.md section 2).
//!
//! The tracker is a plain value that is told the time, so the policy is tested without sleeping.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How a service is restarted after it dies by itself. The default is the one in SERVICES section 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartPolicy {
    /// Automatic restarts allowed within `window`. The next crash after that leaves the service `Crashed`.
    pub max_restarts: u32,
    /// How far back crashes count. A service that ran fine for longer starts with a clean slate.
    pub window: Duration,
    /// The pause before the 1st, 2nd, 3rd ... restart. The last value repeats if there are more restarts than values.
    pub backoff: Vec<Duration>,
    /// The process exits by design now and then (Windows `php-cgi` after `PHP_FCGI_MAX_REQUESTS`): start it again
    /// at once, without a pause and without counting it as a crash. Only for a process that was running; a start
    /// that fails still counts.
    pub recycle_on_exit: bool,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 3,
            window: Duration::from_secs(60),
            backoff: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
            recycle_on_exit: false,
        }
    }
}

/// What to do about a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Start it again after this pause.
    Restart { after: Duration },
    /// Too many crashes in a short time: leave it crashed and tell the user.
    GiveUp,
}

/// Remembers recent crashes of one service.
#[derive(Debug)]
pub struct RestartTracker {
    policy: RestartPolicy,
    crashes: VecDeque<Instant>,
}

impl RestartTracker {
    pub fn new(policy: RestartPolicy) -> Self {
        Self {
            policy,
            crashes: VecDeque::new(),
        }
    }

    /// Record a crash at `now` and say what to do.
    pub fn on_crash(&mut self, now: Instant) -> Decision {
        while self
            .crashes
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) > self.policy.window)
        {
            self.crashes.pop_front();
        }
        self.crashes.push_back(now);

        let count = self.crashes.len();
        let allowed = usize::try_from(self.policy.max_restarts).unwrap_or(usize::MAX);
        if count > allowed {
            return Decision::GiveUp;
        }
        let after = self
            .policy
            .backoff
            .get(count - 1)
            .or_else(|| self.policy.backoff.last())
            .copied()
            .unwrap_or(Duration::ZERO);
        Decision::Restart { after }
    }

    /// Crashes still inside the window as of the last one recorded: how many restarts have been used.
    pub fn used(&self) -> u32 {
        u32::try_from(self.crashes.len()).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn the_default_is_three_restarts_in_sixty_seconds_after_one_two_and_four_seconds() {
        let policy = RestartPolicy::default();
        assert_eq!(policy.max_restarts, 3);
        assert_eq!(policy.window, secs(60));
        assert_eq!(policy.backoff, [secs(1), secs(2), secs(4)]);
        assert!(!policy.recycle_on_exit, "only php-cgi opts in");
    }

    #[test]
    fn pauses_grow_and_the_fourth_crash_in_the_window_gives_up() {
        let mut tracker = RestartTracker::new(RestartPolicy::default());
        let start = Instant::now();

        assert_eq!(
            tracker.on_crash(start),
            Decision::Restart { after: secs(1) }
        );
        assert_eq!(
            tracker.on_crash(start + secs(2)),
            Decision::Restart { after: secs(2) }
        );
        assert_eq!(
            tracker.on_crash(start + secs(6)),
            Decision::Restart { after: secs(4) }
        );
        assert_eq!(tracker.used(), 3);
        assert_eq!(tracker.on_crash(start + secs(12)), Decision::GiveUp);
    }

    #[test]
    fn crashes_outside_the_window_are_forgotten() {
        let mut tracker = RestartTracker::new(RestartPolicy::default());
        let start = Instant::now();
        tracker.on_crash(start);
        tracker.on_crash(start + secs(2));
        tracker.on_crash(start + secs(6));

        // The first three are more than 60 s old: a fresh start with the short pause again.
        let later = start + secs(100);
        assert_eq!(
            tracker.on_crash(later),
            Decision::Restart { after: secs(1) }
        );
        assert_eq!(tracker.used(), 1);
    }

    #[test]
    fn the_window_edge_counts_inside_and_one_moment_later_does_not() {
        let policy = RestartPolicy {
            max_restarts: 1,
            window: secs(10),
            backoff: vec![secs(1)],
            recycle_on_exit: false,
        };
        let start = Instant::now();

        let mut inside = RestartTracker::new(policy.clone());
        inside.on_crash(start);
        assert_eq!(inside.on_crash(start + secs(10)), Decision::GiveUp);

        let mut outside = RestartTracker::new(policy);
        outside.on_crash(start);
        let just_after = start + secs(10) + Duration::from_millis(1);
        assert_eq!(
            outside.on_crash(just_after),
            Decision::Restart { after: secs(1) }
        );
    }

    #[test]
    fn with_no_restarts_allowed_the_first_crash_gives_up() {
        let mut tracker = RestartTracker::new(RestartPolicy {
            max_restarts: 0,
            ..RestartPolicy::default()
        });
        assert_eq!(tracker.on_crash(Instant::now()), Decision::GiveUp);
    }

    #[test]
    fn the_last_pause_repeats_when_there_are_more_restarts_than_pauses() {
        let mut tracker = RestartTracker::new(RestartPolicy {
            max_restarts: 5,
            window: secs(600),
            backoff: vec![secs(1), secs(3)],
            recycle_on_exit: false,
        });
        let start = Instant::now();
        let pauses: Vec<Decision> = (0..5).map(|n| tracker.on_crash(start + secs(n))).collect();
        assert_eq!(
            pauses,
            [
                Decision::Restart { after: secs(1) },
                Decision::Restart { after: secs(3) },
                Decision::Restart { after: secs(3) },
                Decision::Restart { after: secs(3) },
                Decision::Restart { after: secs(3) },
            ]
        );
    }

    #[test]
    fn an_empty_backoff_restarts_at_once() {
        let mut tracker = RestartTracker::new(RestartPolicy {
            backoff: Vec::new(),
            ..RestartPolicy::default()
        });
        assert_eq!(
            tracker.on_crash(Instant::now()),
            Decision::Restart {
                after: Duration::ZERO
            }
        );
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_panic() {
        let mut tracker = RestartTracker::new(RestartPolicy::default());
        let now = Instant::now();
        tracker.on_crash(now + secs(5));
        assert_eq!(tracker.on_crash(now), Decision::Restart { after: secs(2) });
    }
}
