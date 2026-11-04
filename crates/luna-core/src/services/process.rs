//! Starting, probing and stopping one process. No state and no policy here: the supervisor decides.

use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use serde_json::json;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use super::containment::Containment;
use super::os::{self, Leave};
use super::spec::{HelperCommand, ReadyCheck, ServiceSpec, StopMethod, StopPlan};
use crate::error::{ErrorCode, LunaError, Result};

/// One TCP connection attempt of a ready check. Short on purpose: it is always loopback, and on Windows a
/// connection to a port that is not open yet takes about half a second to be refused, so a long timeout would
/// make a service that is up wait for the attempt in flight instead of being noticed on the next one.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
/// A ready-check helper (`mysqladmin ping`) that takes longer than this counts as "not ready yet".
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);
/// After the process was killed, how long to wait for the OS to report it gone.
const KILL_WAIT: Duration = Duration::from_secs(5);

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

/// Make our own child leave: ask the way `plan` says, wait up to the grace period, then kill it. Returns when
/// the process is gone and has been reaped.
pub(super) async fn stop_child(child: &mut Child, plan: &StopPlan) {
    match &plan.method {
        StopMethod::Kill => {
            let _ = child.kill().await;
            return;
        }
        StopMethod::Terminate => ask_child(child, Leave::Terminate),
        StopMethod::Quit => ask_child(child, Leave::Quit),
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

fn ask_child(child: &Child, how: Leave) {
    if let Some(pid) = child.id() {
        os::ask_to_leave(pid, how);
    }
}

/// Kill our own child without asking (a start that timed out).
pub(super) async fn kill_now(child: &mut Child) {
    let _ = child.kill().await;
}

/// Make a process that is **not our child** leave (one adopted after a crash of lunaRun, or found left over):
/// the same plan, but with the process number only, so "gone" is judged by asking the OS.
pub(super) async fn stop_pid(pid: u32, plan: &StopPlan) {
    match &plan.method {
        StopMethod::Kill => {}
        StopMethod::Terminate => os::ask_to_leave(pid, Leave::Terminate),
        StopMethod::Quit => os::ask_to_leave(pid, Leave::Quit),
        StopMethod::Command(helper) => {
            let _ = timeout(plan.grace, run_helper(helper)).await;
        }
    }
    let grace = if matches!(plan.method, StopMethod::Kill) {
        Duration::ZERO
    } else {
        plan.grace
    };
    if !os::wait_gone(pid, grace).await {
        os::terminate_process(pid);
        os::wait_gone(pid, KILL_WAIT).await;
    }
}
