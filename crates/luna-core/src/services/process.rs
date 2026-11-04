//! Starting, probing and stopping one child process. No state and no policy here: the supervisor decides.

use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use serde_json::json;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use super::spec::{HelperCommand, ReadyCheck, ServiceSpec, StopMethod, StopPlan};
use crate::error::{ErrorCode, LunaError, Result};

/// One TCP connection attempt of a ready check. Short on purpose: it is always loopback, and on Windows a
/// connection to a port that is not open yet takes about half a second to be refused, so a long timeout would
/// make a service that is up wait for the attempt in flight instead of being noticed on the next one.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
/// A ready-check helper (`mysqladmin ping`) that takes longer than this counts as "not ready yet".
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

/// Where children are kept from outliving lunaRun (FR-SV-07). The supervisor calls it around every launch.
///
/// The Windows Job Object (`KILL_ON_JOB_CLOSE`) and the Unix process group are the next M1.4 task and plug in
/// here: `before_spawn` puts the child in its own group, `after_spawn` assigns it to the job. Until then
/// [`NoContainment`] does nothing, and the only safety net is that a child is killed when its task is dropped.
pub trait Containment: Send + Sync + 'static {
    /// Adjust the command before it is spawned (for example `process_group(0)`).
    fn before_spawn(&self, command: &mut Command) {
        let _ = command;
    }

    /// Called once with the live child (for example to assign it to a Job Object). An error stops the child.
    fn after_spawn(&self, child: &Child) -> Result<()> {
        let _ = child;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoContainment;

impl Containment for NoContainment {}

/// Windows: do not open a console window for a background service.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn base_command(program: &std::path::Path, args: &[String], env: &[(String, String)]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        // Output goes nowhere until log capture (M1.4 task 3) replaces this.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // The last safety net: if the task that owns the child is dropped, the child dies with it.
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// Launch the service's process.
pub(super) fn spawn(spec: &ServiceSpec, containment: &dyn Containment) -> Result<Child> {
    let mut command = base_command(&spec.program, &spec.args, &spec.env);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    containment.before_spawn(&mut command);
    let mut child = command.spawn().map_err(|err| {
        LunaError::new(
            ErrorCode::ServiceStartFailed,
            format!("{} could not be started: {err}", spec.id),
        )
        .with_details(json!({ "service": spec.id, "program": spec.program.display().to_string() }))
    })?;
    if let Err(err) = containment.after_spawn(&child) {
        // Not contained means it could outlive lunaRun: do not keep it.
        let _ = child.start_kill();
        return Err(err);
    }
    Ok(child)
}

/// Run a helper to completion (the caller bounds the time).
async fn run_helper(helper: &HelperCommand) -> std::io::Result<ExitStatus> {
    base_command(&helper.program, &helper.args, &helper.env)
        .status()
        .await
}

/// One ready-check attempt.
async fn probe(check: &ReadyCheck) -> bool {
    match check {
        ReadyCheck::Immediately => true,
        ReadyCheck::Tcp { port } => {
            timeout(CONNECT_TIMEOUT, TcpStream::connect(("127.0.0.1", *port)))
                .await
                .is_ok_and(|connected| connected.is_ok())
        }
        ReadyCheck::Command(helper) => matches!(
            timeout(HELPER_TIMEOUT, run_helper(helper)).await,
            Ok(Ok(status)) if status.success()
        ),
    }
}

/// Probe every `poll` until the check passes or `limit` is over. `false` means it ran out of time.
pub(super) async fn wait_ready(check: &ReadyCheck, limit: Duration, poll: Duration) -> bool {
    timeout(limit, async {
        while !probe(check).await {
            sleep(poll).await;
        }
    })
    .await
    .is_ok()
}

/// Make the process leave: ask the way `plan` says, wait up to the grace period, then kill it. Returns when the
/// process is gone and has been reaped.
pub(super) async fn stop_child(child: &mut Child, plan: &StopPlan) {
    match &plan.method {
        StopMethod::Kill => {
            let _ = child.kill().await;
            return;
        }
        StopMethod::Terminate => ask_to_leave(child, Signal::Terminate),
        StopMethod::Quit => ask_to_leave(child, Signal::Quit),
        StopMethod::Command(helper) => {
            // The helper may fail or hang (the service may already be gone); either way the grace period
            // below decides, so its result does not matter.
            let _ = timeout(plan.grace, run_helper(helper)).await;
        }
    }
    if timeout(plan.grace, child.wait()).await.is_err() {
        // `kill` waits for the process to be gone.
        let _ = child.kill().await;
    }
}

/// Kill without asking (a start that timed out).
pub(super) async fn kill_now(child: &mut Child) {
    let _ = child.kill().await;
}

#[derive(Clone, Copy)]
enum Signal {
    Terminate,
    Quit,
}

#[cfg(unix)]
fn ask_to_leave(child: &mut Child, signal: Signal) {
    // Both numbers are the same on Linux and macOS.
    let number = match signal {
        Signal::Terminate => 15,
        Signal::Quit => 3,
    };
    if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
        // An error means the process is already gone, which is what we wanted.
        let _ = kill(pid, number);
    }
}

#[cfg(unix)]
unsafe extern "C" {
    /// POSIX `kill(2)`, which `std` already links. Safe to call with any numbers: a bad pid is an error code.
    safe fn kill(pid: i32, signal: i32) -> i32;
}

/// Windows has no polite signal for a background process without a window: terminate it.
#[cfg(windows)]
fn ask_to_leave(child: &mut Child, _signal: Signal) {
    let _ = child.start_kill();
}
