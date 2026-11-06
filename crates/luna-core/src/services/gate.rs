//! One lock between "a probe socket is open" and "a process is being created".
//!
//! On Linux and macOS a process is created by forking. For the moment between the fork and the child's `exec`, the child
//! holds a copy of **every** file descriptor lunaRun has open, among them the socket another thread has just opened to
//! test whether a port is free (`check_ports`, the allocator's scan). The probe is closed a microsecond later, but the
//! child's copy keeps the port **listening** until it has exec'd, so a check made right then finds a listener that
//! nobody owns and refuses the port with `PORT_IN_USE` (seen on Linux CI, never on Windows, where sockets are not
//! inherited). Starting several services at once, as "Start all" does, makes it likely.
//!
//! Holding this lock while a probe is open and while a process is spawned keeps the two apart. `Command::spawn` returns
//! only after the child has exec'd (and so closed its inherited copies), so the lock is held exactly as long as it needs
//! to be. It is never held across an `.await`.

use std::sync::{Mutex, MutexGuard, PoisonError};

static GATE: Mutex<()> = Mutex::new(());

/// Take the lock. A panic elsewhere must not stop services from starting, so a poisoned lock is simply used.
pub(super) fn hold() -> MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}
