//! Keeping children from outliving lunaRun (FR-SV-07, docs/SERVICES.md "No orphans").
//!
//! - **Windows:** every service gets its own **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The OS
//!   closes lunaRun's handle when lunaRun dies, however it dies, and kills everything in the job. When a service
//!   ends normally, dropping its job kills whatever it left behind.
//! - **macOS and Linux:** every service is started in its **own process group**, so everything it starts is in
//!   that group, and when the service ends the whole group is killed. If lunaRun itself crashes the children may
//!   survive: PID files (`pidfile.rs`) are how the next launch finds them.
//!
//! One small gap on Windows: a process is put in its job right after it starts, so a child it started in those
//! microseconds would be outside the job. Closing it needs `CREATE_SUSPENDED`, which the standard library does not
//! offer, and none of the services start children before they have finished starting.

use tokio::process::{Child, Command};

use crate::error::Result;

/// Where children are contained. The supervisor calls it around every launch.
pub trait Containment: Send + Sync + 'static {
    /// Adjust the command before it is spawned (for example `process_group(0)`).
    fn before_spawn(&self, command: &mut Command) {
        let _ = command;
    }

    /// Called once with the live child (for example to assign it to a Job Object). An error stops the child:
    /// one that is not contained could outlive lunaRun.
    fn after_spawn(&self, child: &Child) -> Result<()> {
        let _ = child;
        Ok(())
    }

    /// The process `pid` has ended (or was stopped and reaped): kill whatever it left behind.
    fn after_exit(&self, pid: u32) {
        let _ = pid;
    }
}

/// Contains nothing. For tests that must see what happens without containment.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoContainment;

impl Containment for NoContainment {}

#[cfg(unix)]
pub use groups::ProcessGroups as OsContainment;
#[cfg(windows)]
pub use jobs::JobObjects as OsContainment;

#[cfg(unix)]
mod groups {
    use std::os::unix::process::CommandExt;

    use tokio::process::Command;

    use super::Containment;
    use crate::services::os;

    /// One process group per service.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct ProcessGroups;

    impl ProcessGroups {
        pub fn new() -> Self {
            Self
        }
    }

    impl Containment for ProcessGroups {
        fn before_spawn(&self, command: &mut Command) {
            // A new group whose id is the child's pid.
            command.as_std_mut().process_group(0);
        }

        fn after_exit(&self, pid: u32) {
            os::kill_group(pid);
        }
    }
}

#[cfg(windows)]
mod jobs {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::{Mutex, PoisonError};

    use serde_json::json;
    use tokio::process::Child;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    use super::Containment;
    use crate::error::{ErrorCode, LunaError, Result};

    /// One Job Object per service, kept until the service ends. Dropping the handle closes the job, and a job
    /// closed with `KILL_ON_JOB_CLOSE` kills every process in it.
    #[derive(Debug, Default)]
    pub struct JobObjects {
        jobs: Mutex<HashMap<u32, OwnedHandle>>,
    }

    impl JobObjects {
        pub fn new() -> Self {
            Self::default()
        }
    }

    fn failed(what: &str) -> LunaError {
        LunaError::new(
            ErrorCode::ServiceStartFailed,
            format!(
                "could not put the service in a Job Object ({what}): {}",
                io::Error::last_os_error()
            ),
        )
        .with_details(json!({ "step": what }))
    }

    /// A job that kills its processes when its last handle closes.
    fn new_job() -> Result<OwnedHandle> {
        // SAFETY: both arguments are optional and null here (default security, no name).
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(failed("create"));
        }
        // SAFETY: `raw` is a valid handle that we own from here on.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };

        // SAFETY: an all-zero value is a valid "no limits" `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(|_| failed("size"))?;
        // SAFETY: the pointer and size describe `info`, which lives for the call.
        let ok = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast::<c_void>(),
                size,
            )
        };
        if ok == 0 {
            return Err(failed("limits"));
        }
        Ok(job)
    }

    impl Containment for JobObjects {
        fn after_spawn(&self, child: &Child) -> Result<()> {
            let (Some(pid), Some(process)) = (child.id(), child.raw_handle()) else {
                // The process is already gone: nothing to contain.
                return Ok(());
            };
            let job = new_job()?;
            // SAFETY: both handles are valid for the call.
            if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
                return Err(failed("assign"));
            }
            self.jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(pid, job);
            Ok(())
        }

        fn after_exit(&self, pid: u32) {
            // Dropping the handle closes the job and kills what the service left running.
            self.jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&pid);
        }
    }
}
