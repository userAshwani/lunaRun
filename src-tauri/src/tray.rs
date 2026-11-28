//! The tray icon and its menu (FR-AP-01, FR-AP-02; D-043).
//!
//! lunaRun lives in the system tray (the Windows notification area, the macOS menu bar, a Linux status area): **closing
//! the window hides it** and the services keep running; **Quit** in the menu stops them in the order of D-029 and exits.
//! When no tray icon can be made (a Linux desktop without a status area), closing the window quits as it did before, so
//! services are never left running with nothing on screen to stop them.
//!
//! [`menu_model`] is pure (the projects, the sites and how many services run, in; the items, out) and tested; this
//! module only turns it into Tauri's menu, rebuilds it when a service or a job changes, and maps a click to what the
//! page's own commands do.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use luna_core::LunaHome;
use luna_core::jobs::JobManager;
use luna_core::launch::{ProjectAction, ProjectLauncher};
use luna_core::opener::OpenTarget;
use luna_core::projects::ProjectStore;
use luna_core::services::{ServiceState, Supervisor};
use luna_core::sites::{SiteManager, SiteState};
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::services::{QuitState, spawn_quit};

/// The argument lunaRun is started with at login: it opens in the tray, with no window (D-045).
pub const HIDDEN_ARG: &str = "--hidden";

/// The id of lunaRun's one tray icon.
pub const TRAY_ID: &str = "lunarun";

/// The English words of the tray. The menu is drawn by the operating system, not by the page, so it cannot use the page's
/// i18n files; this table is the Rust side's, keyed like `en.ts` (`tray.*`), and a translation is another `TrayText`.
#[derive(Debug, Clone, Copy)]
pub struct TrayText {
    pub tooltip: &'static str,
    pub nothing_running: &'static str,
    pub one_running: &'static str,
    /// `{count}` is replaced.
    pub many_running: &'static str,
    pub open: &'static str,
    pub projects: &'static str,
    pub no_projects: &'static str,
    pub state_running: &'static str,
    pub state_not_answering: &'static str,
    pub start: &'static str,
    pub stop: &'static str,
    pub browser: &'static str,
    pub terminal: &'static str,
    pub stop_all: &'static str,
    pub quit: &'static str,
}

pub const EN: TrayText = TrayText {
    tooltip: "lunaRun",
    nothing_running: "Nothing is running",
    one_running: "1 service is running",
    many_running: "{count} services are running",
    open: "Open lunaRun",
    projects: "Projects",
    no_projects: "No projects yet",
    state_running: "running",
    state_not_answering: "not answering",
    start: "Start project",
    stop: "Stop project",
    browser: "Open in browser",
    terminal: "Open terminal",
    stop_all: "Stop all services",
    quit: "Quit lunaRun",
};

/// What a menu item does. Its id is the text form ([`TrayAction::id`]), so a click maps back without a lookup table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    StopAll,
    Quit,
    Start(String),
    Stop(String),
    Browser(String),
    Terminal(String),
}

impl TrayAction {
    pub fn id(&self) -> String {
        match self {
            Self::Open => "open".to_owned(),
            Self::StopAll => "stop-all".to_owned(),
            Self::Quit => "quit".to_owned(),
            Self::Start(id) => format!("start:{id}"),
            Self::Stop(id) => format!("stop:{id}"),
            Self::Browser(id) => format!("browser:{id}"),
            Self::Terminal(id) => format!("terminal:{id}"),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "open" => return Some(Self::Open),
            "stop-all" => return Some(Self::StopAll),
            "quit" => return Some(Self::Quit),
            _ => {}
        }
        let (kind, id) = text.split_once(':')?;
        if id.is_empty() {
            return None;
        }
        let id = id.to_owned();
        match kind {
            "start" => Some(Self::Start(id)),
            "stop" => Some(Self::Stop(id)),
            "browser" => Some(Self::Browser(id)),
            "terminal" => Some(Self::Terminal(id)),
            _ => None,
        }
    }
}

/// One entry of the menu, before Tauri makes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayItem {
    /// A line that says something and does nothing.
    Text(String),
    Action {
        label: String,
        action: TrayAction,
        enabled: bool,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<TrayItem>,
    },
}

/// What the menu needs to know about one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayProject {
    pub id: String,
    pub name: String,
    /// It can be started: its PHP is installed and its folder is there.
    pub has_site: bool,
    /// The user started it.
    pub served: bool,
    /// Started, and every service it needs runs.
    pub running: bool,
    pub address: Option<String>,
}

/// The status line: how many services run.
pub fn status_line(running_services: usize, text: &TrayText) -> String {
    match running_services {
        0 => text.nothing_running.to_owned(),
        1 => text.one_running.to_owned(),
        n => text.many_running.replace("{count}", &n.to_string()),
    }
}

/// The menu: the status, Open lunaRun, the projects (each with Start or Stop, Open in browser, Open terminal), Stop all
/// services, Quit. Only what can be done is enabled.
pub fn menu_model(
    projects: &[TrayProject],
    running_services: usize,
    text: &TrayText,
) -> Vec<TrayItem> {
    let action = |label: &str, action: TrayAction, enabled: bool| TrayItem::Action {
        label: label.to_owned(),
        action,
        enabled,
    };
    let mut items = vec![
        TrayItem::Text(status_line(running_services, text)),
        action(text.open, TrayAction::Open, true),
        TrayItem::Separator,
    ];
    if projects.is_empty() {
        items.push(TrayItem::Text(text.no_projects.to_owned()));
    } else {
        items.push(TrayItem::Text(text.projects.to_owned()));
        for project in projects {
            let label = if project.running {
                format!("{} ({})", project.name, text.state_running)
            } else if project.served {
                format!("{} ({})", project.name, text.state_not_answering)
            } else {
                project.name.clone()
            };
            let id = project.id.clone();
            let start_or_stop = if project.served {
                action(text.stop, TrayAction::Stop(id.clone()), true)
            } else {
                action(text.start, TrayAction::Start(id.clone()), project.has_site)
            };
            items.push(TrayItem::Submenu {
                label,
                items: vec![
                    start_or_stop,
                    action(
                        text.browser,
                        TrayAction::Browser(id.clone()),
                        project.running && project.address.is_some(),
                    ),
                    action(text.terminal, TrayAction::Terminal(id), project.has_site),
                ],
            });
        }
    }
    items.push(TrayItem::Separator);
    items.push(action(
        text.stop_all,
        TrayAction::StopAll,
        running_services > 0,
    ));
    items.push(action(text.quit, TrayAction::Quit, true));
    items
}

// ---------------------------------------------------------------------------------------------
// Tauri
// ---------------------------------------------------------------------------------------------

/// What the menu shows, read from the app's state now.
fn current_model<R: Runtime>(app: &AppHandle<R>) -> Vec<TrayItem> {
    let running = app
        .state::<Supervisor>()
        .list()
        .iter()
        .filter(|status| status.state == ServiceState::Running)
        .count();
    let entries = app
        .state::<SiteManager>()
        .plan()
        .map(|plan| plan.entries)
        .unwrap_or_default();
    let projects: Vec<TrayProject> = app
        .state::<ProjectStore>()
        .list()
        .into_iter()
        .map(|project| {
            let entry = entries.iter().find(|e| e.project_id == project.id);
            TrayProject {
                has_site: entry.is_some_and(|e| e.state == SiteState::Ready),
                served: entry.is_some_and(|e| e.served),
                running: entry.is_some_and(|e| e.running),
                address: entry.and_then(|e| e.address.clone()),
                id: project.id,
                name: project.name,
            }
        })
        .collect();
    menu_model(&projects, running, &EN)
}

fn build_items<R: Runtime>(
    app: &AppHandle<R>,
    model: &[TrayItem],
) -> tauri::Result<Vec<Box<dyn IsMenuItem<R>>>> {
    let mut built: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
    for item in model {
        match item {
            TrayItem::Text(label) => {
                built.push(Box::new(MenuItem::new(app, label, false, None::<&str>)?));
            }
            TrayItem::Action {
                label,
                action,
                enabled,
            } => {
                built.push(Box::new(MenuItem::with_id(
                    app,
                    action.id(),
                    label,
                    *enabled,
                    None::<&str>,
                )?));
            }
            TrayItem::Separator => built.push(Box::new(PredefinedMenuItem::separator(app)?)),
            TrayItem::Submenu { label, items } => {
                let children = build_items(app, items)?;
                let refs: Vec<&dyn IsMenuItem<R>> = children.iter().map(AsRef::as_ref).collect();
                built.push(Box::new(Submenu::with_items(app, label, true, &refs)?));
            }
        }
    }
    Ok(built)
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let items = build_items(app, &current_model(app))?;
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(AsRef::as_ref).collect();
    Menu::with_items(app, &refs)
}

/// Bring the window back (from the tray, or after it was hidden by closing it).
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Is there a tray icon? Without one, closing the window must quit (nothing else would be left to quit with).
pub fn is_present<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}

/// Rebuild the menu soon. Many changes come at once (a project start changes three services), so they are gathered for a
/// moment and the menu is rebuilt once.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    static PENDING: AtomicBool = AtomicBool::new(false);
    if !is_present(app) || PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        PENDING.store(false, Ordering::SeqCst);
        rebuild(&app);
    });
}

/// Rebuild the menu now (the pointer is on the icon: the menu may be opened next).
fn rebuild<R: Runtime>(app: &AppHandle<R>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let running = app
        .state::<Supervisor>()
        .list()
        .iter()
        .filter(|status| status.state == ServiceState::Running)
        .count();
    if let Ok(menu) = build_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_tooltip(Some(format!(
        "{}: {}",
        EN.tooltip,
        status_line(running, &EN)
    )));
}

/// Quit as the window's close used to: stop every service in order (with the window shown, so the progress is seen when
/// something has to be stopped), then exit.
fn quit<R: Runtime>(app: &AppHandle<R>) {
    let busy = app
        .state::<Supervisor>()
        .list()
        .iter()
        .any(|status| status.state.has_process());
    if busy {
        show_window(app);
    }
    if app.state::<QuitState>().begin() {
        spawn_quit(app, 0);
    }
}

/// Stop the started projects (each by its own stop, which takes it out of Caddy and stops only what its start began), then
/// everything else that still runs, in the order of a quit. lunaRun stays open.
fn stop_all<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let launcher = app.state::<ProjectLauncher>().inner().clone();
        let jobs = app.state::<JobManager>().inner().clone();
        let served: Vec<String> = app
            .state::<SiteManager>()
            .plan()
            .map(|plan| plan.entries)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.served)
            .map(|entry| entry.project_id)
            .collect();
        for id in served {
            if let Ok(started) = launcher.run(&jobs, &id, ProjectAction::Stop) {
                let _ = jobs.wait(&started.id).await;
            }
        }
        let supervisor = app.state::<Supervisor>().inner().clone();
        let _ = supervisor
            .shutdown_all(luna_core::services::DEFAULT_GROUP_LIMIT, |_| {})
            .await;
        refresh(&app);
    });
}

fn on_action<R: Runtime>(app: &AppHandle<R>, action: TrayAction) {
    match action {
        TrayAction::Open => show_window(app),
        TrayAction::Quit => quit(app),
        TrayAction::StopAll => stop_all(app),
        TrayAction::Start(id) => run_project(app, &id, ProjectAction::Start),
        TrayAction::Stop(id) => run_project(app, &id, ProjectAction::Stop),
        TrayAction::Browser(id) => open(app, id, OpenTarget::Browser),
        TrayAction::Terminal(id) => open(app, id, OpenTarget::Terminal),
    }
}

/// Start or stop a project as the page's button does: a job the page shows when it is opened. A refusal (another action
/// on it is running) brings the window up on the project, where the reason is shown.
fn run_project<R: Runtime>(app: &AppHandle<R>, id: &str, action: ProjectAction) {
    if app.state::<QuitState>().is_quitting() {
        return;
    }
    let launcher = app.state::<ProjectLauncher>();
    let jobs = app.state::<JobManager>();
    if launcher.run(&jobs, id, action).is_err() {
        show_window(app);
    }
}

fn open<R: Runtime>(app: &AppHandle<R>, id: String, target: OpenTarget) {
    let home = app.state::<LunaHome>().inner().clone();
    let projects = app.state::<ProjectStore>().inner().clone();
    let sites = app.state::<SiteManager>().inner().clone();
    let window_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // A failure (no PHP for a terminal, no site for the browser) is explained on the project page.
        if crate::commands::open_project(&home, &projects, &sites, &id, target).is_err() {
            show_window(&window_app);
        }
    });
}

/// Make the tray icon. `false` when the system has no tray (then closing the window quits, as before).
pub fn create<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(icon) = app.default_window_icon().cloned() else {
        return false;
    };
    let Ok(menu) = build_menu(app) else {
        return false;
    };
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(format!("{}: {}", EN.tooltip, EN.nothing_running))
        .menu(&menu)
        // A left click opens the window on Windows and Linux; on macOS the menu bar shows the menu, as its users expect.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| {
            if let Some(action) = TrayAction::parse(event.id().as_ref()) {
                on_action(app, action);
            }
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } if !cfg!(target_os = "macos") => show_window(tray.app_handle()),
            // The pointer reached the icon: the menu may be opened next, so it shows what is true now (a project added
            // on the page, for example, changes no service and would otherwise wait for the next change).
            TrayIconEvent::Enter { .. } => rebuild(tray.app_handle()),
            _ => {}
        })
        .build(app)
        .is_ok()
}
