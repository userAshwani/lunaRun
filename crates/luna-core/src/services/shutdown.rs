//! Stopping everything when lunaRun quits (docs/SERVICES.md section 2 "Graceful stop"; D-029).
//!
//! [`Supervisor::shutdown_all`] stops every service that is not already stopped, in three groups and in this order:
//!
//! 1. **the web side** (`caddy`, and anything this does not know): nothing sends requests to PHP any more,
//! 2. **PHP** (`php-fcgi@...`, `php-fpm@...`): nothing sends queries to a database any more,
//! 3. **the databases** (`mysql@...`, `mariadb@...`): they get the quiet moment they need for a clean shutdown.
//!
//! Inside a group everything stops **at the same time**, so three services take about one grace period, not three.
//! The next group starts only when the one before is **fully stopped** (or ran out of time), so no query is in
//! flight when a database shuts down. Nothing stops at the first problem: every service gets its turn, and the
//! [`ShutdownReport`] says what left by itself, what had to be killed (a database then does crash recovery on its
//! next start) and what could not be stopped.
//!
//! The caller must not start services while this runs. On Windows a **hard kill** of lunaRun ends every service at
//! once through the Job Objects (D-024), with no order: this is for a normal Quit.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use ts_rs::TS;

use super::state::ServiceState;
use super::supervisor::Supervisor;
use crate::error::{ErrorCode, LunaError};

/// How long one group gets in all before the next one starts anyway: the longest grace period of a service (a
/// database's 30 s) with room for it to finish writing and for lunaRun to notice.
pub const DEFAULT_GROUP_LIMIT: Duration = Duration::from_secs(60);

/// The three groups of a Quit, in the order they are stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ShutdownGroup {
    /// `caddy`, and every id this does not know.
    Web,
    /// `php-fcgi@...` and `php-fpm@...`.
    Php,
    /// `mysql@...` and `mariadb@...`.
    Databases,
}

impl ShutdownGroup {
    /// The order of a Quit.
    pub const ORDER: [Self; 3] = [Self::Web, Self::Php, Self::Databases];
}

/// The group a service id belongs to. Ids come from the service definitions (`caddy`, `php-fcgi@8.3.36-w1`,
/// `php-fpm@8.3.36`, `mysql@8.4`, `mariadb@12.3`); an id nobody knows is stopped with the first group, before
/// anything that may depend on it is taken away.
pub fn group_of(id: &str) -> ShutdownGroup {
    if id.starts_with("php-fcgi@") || id.starts_with("php-fpm@") {
        ShutdownGroup::Php
    } else if id.starts_with("mysql@") || id.starts_with("mariadb@") {
        ShutdownGroup::Databases
    } else {
        ShutdownGroup::Web
    }
}

/// Where a Quit is, for the progress window. Sent when a group starts and each time a service of it is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShutdownProgress {
    pub group: ShutdownGroup,
    /// How many services this group stops, and how many are done.
    pub total: u32,
    pub done: u32,
    /// The ids still being stopped, sorted.
    pub remaining: Vec<String>,
    /// The service that has just finished; `None` when the group has just started.
    pub finished: Option<String>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

/// A service that could not be stopped, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShutdownFailure {
    pub id: String,
    pub error: LunaError,
}

/// What a Quit did, in the order the services finished.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShutdownReport {
    /// Left by themselves, within their grace period.
    pub clean: Vec<String>,
    /// Did not leave in time and were killed. A database among them does crash recovery on its next start.
    pub killed: Vec<String>,
    /// Could not be stopped, or were still not stopped when the group ran out of time (`SERVICE_STOP_FAILED`).
    pub failed: Vec<ShutdownFailure>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn not_stopped(id: &str, why: &str) -> LunaError {
    LunaError::new(
        ErrorCode::ServiceStopFailed,
        format!("{id} could not be stopped: {why}"),
    )
    .with_details(serde_json::json!({ "service": id }))
}

impl Supervisor {
    /// Stop every service that is not stopped, group by group (see the module documentation). `group_limit` is how
    /// long one group gets in all ([`DEFAULT_GROUP_LIMIT`]); what is still running then is reported as failed and the
    /// next group goes ahead. `on_progress` is called when a group starts and whenever one of its services is done.
    ///
    /// Services that are `Crashed` or `Error` are stopped too: a crashed service may be waiting to restart itself, and
    /// that would start a process in the middle of a Quit. Harmless and quick when nothing runs.
    pub async fn shutdown_all(
        &self,
        group_limit: Duration,
        mut on_progress: impl FnMut(&ShutdownProgress),
    ) -> ShutdownReport {
        let begun = Instant::now();
        let mut report = ShutdownReport::default();
        for group in ShutdownGroup::ORDER {
            let ids: Vec<String> = self
                .list()
                .into_iter()
                .filter(|status| status.state != ServiceState::Stopped)
                .map(|status| status.id)
                .filter(|id| group_of(id) == group)
                .collect();
            if !ids.is_empty() {
                self.stop_group(
                    group,
                    ids,
                    group_limit,
                    begun,
                    &mut on_progress,
                    &mut report,
                )
                .await;
            }
        }
        report.elapsed_ms = millis(begun);
        report
    }

    async fn stop_group(
        &self,
        group: ShutdownGroup,
        ids: Vec<String>,
        limit: Duration,
        begun: Instant,
        on_progress: &mut impl FnMut(&ShutdownProgress),
        report: &mut ShutdownReport,
    ) {
        let total = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        let mut remaining: BTreeSet<String> = ids.iter().cloned().collect();
        let mut done = 0_u32;
        let mut announce = |finished: Option<String>, remaining: &BTreeSet<String>, done: u32| {
            on_progress(&ShutdownProgress {
                group,
                total,
                done,
                remaining: remaining.iter().cloned().collect(),
                finished,
                elapsed_ms: millis(begun),
            });
        };
        announce(None, &remaining, done);

        let mut pending: FuturesUnordered<_> = ids
            .into_iter()
            .map(|id| {
                let supervisor = self.clone();
                async move {
                    let result = supervisor.stop(&id).await;
                    (id, result)
                }
            })
            .collect();
        let deadline = sleep(limit);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                next = pending.next() => {
                    let Some((id, result)) = next else { break };
                    remaining.remove(&id);
                    done += 1;
                    match result {
                        Ok(()) if self.status(&id).is_some_and(|status| status.killed) => {
                            report.killed.push(id.clone());
                        }
                        Ok(()) => report.clean.push(id.clone()),
                        Err(error) => report.failed.push(ShutdownFailure {
                            id: id.clone(),
                            error: not_stopped(&id, &error.message),
                        }),
                    }
                    announce(Some(id), &remaining, done);
                }
                () = &mut deadline => {
                    // Their stop stays asked for; this just stops waiting, so the Quit goes on.
                    for id in std::mem::take(&mut remaining) {
                        let why = format!("it was still running after {} s", limit.as_secs());
                        report.failed.push(ShutdownFailure {
                            error: not_stopped(&id, &why),
                            id,
                        });
                    }
                    break;
                }
            }
        }
    }
}
