//! The services as the running app holds them: the supervisor, the port allocator, what was found still running from an
//! earlier session, and the orderly stop at quit. The logic is in `luna-core` (`services`); this puts it into Tauri's
//! state and sends its events to the webview.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use luna_core::LunaHome;
use luna_core::diagnostics::Os;
use luna_core::runtimes::Uninstaller;
use luna_core::services::{
    DEFAULT_GROUP_LIMIT, Leftover, PortAllocator, ServiceController, ShutdownReport, Supervisor,
    usage_checker,
};
use tauri::{AppHandle, Manager, Runtime};

use crate::events::{emit_service_log, emit_service_state, emit_shutdown_progress};

/// Services that were still running when the app started, left by an earlier session that did not stop them (a crash;
/// on Windows the Job Objects end them with lunaRun, so this is a macOS and Linux case). Found **once**, at startup
/// (FR-SV-07). The page lists them; adopting or stopping one removes it from here.
#[derive(Default)]
pub struct Leftovers(Mutex<Vec<Leftover>>);

impl Leftovers {
    pub fn new(found: Vec<Leftover>) -> Self {
        Self(Mutex::new(found))
    }

    /// A copy of the list as it is now.
    pub fn list(&self) -> Vec<Leftover> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Where a quit is. The first request starts the orderly stop; while it runs further requests change nothing; when it is
/// over the quit goes through. Services must not be started while [`is_quitting`](Self::is_quitting) (D-029).
#[derive(Default)]
pub struct QuitState {
    began: AtomicBool,
    finished: AtomicBool,
}

impl QuitState {
    /// `true` for the first caller only: that one starts the stop.
    pub fn begin(&self) -> bool {
        !self.began.swap(true, Ordering::SeqCst)
    }

    pub fn is_quitting(&self) -> bool {
        self.began.load(Ordering::SeqCst)
    }

    pub fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

/// Create the supervisor and the port allocator, look **once** for services left from an earlier session, and put all of
/// it into the app's state, together with the uninstaller (which asks the supervisor what runs from a version, so it
/// refuses to remove one that is in use) and the quit state. Returns the supervisor and the allocator for whoever needs
/// them while the app is being set up.
pub fn manage<R: Runtime>(app: &AppHandle<R>, home: &LunaHome) -> (Supervisor, PortAllocator) {
    let state_app = app.clone();
    let log_app = app.clone();
    let supervisor = Supervisor::new(
        tauri::async_runtime::handle().inner().clone(),
        home.clone(),
        move |status| emit_service_state(&state_app, &status),
    )
    .with_log_sink(move |batch| emit_service_log(&log_app, &batch));
    let ports = PortAllocator::new(home.clone());

    app.manage(Leftovers::new(supervisor.scan_leftovers()));
    app.manage(QuitState::default());
    app.manage(Uninstaller::new(home.clone()).with_usage(usage_checker(
        ports.clone(),
        supervisor.clone(),
        home.clone(),
        Os::current(),
    )));
    // Starting, stopping and restarting by the id of a row. Nothing may be started once the quit has begun.
    let quit_app = app.clone();
    app.manage(
        ServiceController::new(
            home.clone(),
            ports.clone(),
            supervisor.clone(),
            Os::current(),
        )
        .with_quit_check(move || quit_app.state::<QuitState>().is_quitting()),
    );
    app.manage(ports.clone());
    app.manage(supervisor.clone());
    (supervisor, ports)
}

/// Stop every service in the order of a quit (web, then PHP, then databases), telling the webview how far it has come,
/// and mark the quit as finished. Nothing here exits the app: the caller does that.
pub async fn run_quit<R: Runtime>(app: &AppHandle<R>) -> ShutdownReport {
    let supervisor = app.state::<Supervisor>().inner().clone();
    let emitter = app.clone();
    let report = supervisor
        .shutdown_all(DEFAULT_GROUP_LIMIT, move |progress| {
            emit_shutdown_progress(&emitter, progress);
        })
        .await;
    app.state::<QuitState>().finish();
    report
}

/// Tauri asks to exit (the last window was closed, or `exit` was called). `true`: let it go. `false`: not yet. The
/// first request starts the orderly stop, which exits for real when it is done; the exit it asks for comes back here
/// and goes through.
pub fn on_exit_requested<R: Runtime>(app: &AppHandle<R>, code: Option<i32>) -> bool {
    let quit = app.state::<QuitState>();
    if quit.is_finished() {
        return true;
    }
    if quit.begin() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            run_quit(&app).await;
            app.exit(code.unwrap_or(0));
        });
    }
    false
}
