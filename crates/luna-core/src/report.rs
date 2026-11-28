//! The Diagnostics page and the report a user copies into a bug report (FR-AP-04, NFR-06; D-046).
//!
//! [`DiagnosticsReport`] holds **facts**, gathered by the caller (the Tauri adapter asks the supervisor, the job list,
//! the port allocator, the system), so this module is pure and tested anywhere. [`report_text`] turns them into plain
//! English text for a GitHub issue, and **everything it writes goes through [`scrub`]**: the user's home folder becomes
//! `~` (it holds the user name) and the value of anything that looks like a password, a token, a secret or a key is
//! replaced by `***`. What is never gathered at all: the contents of `.env` files (lunaRun never reads a password,
//! golden rule 7), project names and folders (only their number), the version list's address (only its host, and only
//! when it is not the built-in one). Nothing is sent anywhere: the user copies the text (no telemetry, D-008).

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;
use ts_rs::TS;

use crate::diagnostics::AppInfo;
use crate::jobs::{JobInfo, JobState};
use crate::onboarding::{HomeSource, VcRedistState};
use crate::registry::{RegistrySourceKind, RegistryStatus};
use crate::services::{PortOwner, ServiceState, ServiceStatus};
use crate::settings::Settings;

/// Ports every PHP developer's machine tends to have taken by something else (XAMPP's Apache and MySQL, another web
/// server, a proxy). They are checked even when lunaRun uses none of them, because a clash there explains many reports.
pub const COMMON_PORTS: [u16; 4] = [80, 443, 3306, 8080];

/// One port that was checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortCheck {
    pub port: u16,
    /// The service or site lunaRun gave this port to (`mysql@8.4`, `site@4b1e0c`), none for a common port.
    pub assigned_to: Option<String>,
    /// Something listens on it.
    pub held: bool,
    /// lunaRun's own program holds it (a service it runs).
    pub by_lunarun: bool,
    /// The holder's program name (`httpd.exe`), when the system says. Never its full path.
    pub owner: Option<String>,
}

/// One service as the supervisor knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceLine {
    pub id: String,
    pub state: ServiceState,
    /// The code and message of its last failure.
    pub error: Option<String>,
}

/// A background task that failed in this session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FailedJob {
    pub label: String,
    pub error: String,
}

/// Where the version list came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RegistryLine {
    pub source: RegistrySourceKind,
    #[ts(type = "number | null")]
    pub age_seconds: Option<u64>,
    pub problem: Option<String>,
    /// The host of a custom address (the Settings page); none for the built-in one. Never the whole address.
    pub custom_host: Option<String>,
}

/// Everything the Diagnostics page shows and the report says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagnosticsReport {
    pub app: AppInfo,
    pub home_source: HomeSource,
    #[ts(type = "number | null")]
    pub free_bytes: Option<u64>,
    pub vc_redist: VcRedistState,
    /// `php 8.6.0`, newest first per runtime.
    pub installed: Vec<String>,
    /// How many projects (never their names or folders).
    pub projects: u32,
    pub ports: Vec<PortCheck>,
    pub services: Vec<ServiceLine>,
    pub failed_jobs: Vec<FailedJob>,
    pub registry: RegistryLine,
    /// The settings that differ from the defaults, as `name = value` (the address as its host only).
    pub changed_settings: Vec<String>,
}

fn error_text(error: &crate::error::LunaError) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!("{code}: {}", error.message)
}

/// What the supervisor knows, as lines: every service that is not simply stopped, and stopped ones with an error.
pub fn service_lines(statuses: &[ServiceStatus]) -> Vec<ServiceLine> {
    statuses
        .iter()
        .filter(|s| s.state != ServiceState::Stopped || s.error.is_some())
        .map(|s| ServiceLine {
            id: s.id.clone(),
            state: s.state,
            error: s.error.as_ref().map(error_text),
        })
        .collect()
}

/// The failed jobs of this session (nothing is stored between sessions), newest first, ten at most.
pub fn failed_jobs(jobs: &[JobInfo]) -> Vec<FailedJob> {
    jobs.iter()
        .filter(|j| j.state == JobState::Failed)
        .take(10)
        .map(|j| FailedJob {
            label: j.label.clone(),
            error: j.error.as_ref().map(error_text).unwrap_or_default(),
        })
        .collect()
}

/// The registry status, with the host of a custom address.
pub fn registry_line(status: &RegistryStatus, custom_url: Option<&str>) -> RegistryLine {
    RegistryLine {
        source: status.source,
        age_seconds: status.age_seconds,
        problem: status.problem.as_ref().map(error_text),
        custom_host: custom_url.and_then(|url| {
            reqwest::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
        }),
    }
}

/// The settings that differ from the defaults (values that are not secret; the address as its host).
pub fn changed_settings(settings: &Settings) -> Vec<String> {
    let defaults = Settings::default();
    let mut changed = Vec::new();
    if let Some(line) = &settings.default_php_line {
        changed.push(format!("default PHP line = {line}"));
    }
    if let Some(line) = &settings.default_node_line {
        changed.push(format!("default Node.js line = {line}"));
    }
    let (p, d) = (settings.ports, defaults.ports);
    for (name, now, default) in [
        ("site ports", p.site, d.site),
        ("PHP ports", p.php_fastcgi, d.php_fastcgi),
        ("database ports", p.database, d.database),
    ] {
        if now != default {
            changed.push(format!("{name} = {}-{}", now.start, now.end));
        }
    }
    if p.caddy_admin != d.caddy_admin {
        changed.push(format!("web server admin port = {}", p.caddy_admin));
    }
    if let Some(url) = &settings.registry_url {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| "an address".to_owned());
        changed.push(format!("version list address = custom ({host})"));
    }
    changed
}

/// One port checked: who holds it, and whether that is lunaRun itself (its program is inside `luna_home`).
pub fn port_check(
    port: u16,
    assigned_to: Option<String>,
    owners: &[PortOwner],
    luna_home: &Path,
) -> PortCheck {
    let ours = |o: &PortOwner| {
        o.process_path
            .as_deref()
            .is_some_and(|p| Path::new(p).starts_with(luna_home))
    };
    PortCheck {
        port,
        assigned_to,
        held: !owners.is_empty(),
        by_lunarun: !owners.is_empty() && owners.iter().all(ours),
        owner: owners.iter().find_map(|o| o.process_name.clone()),
    }
}

/// The page's answer: the facts and the report text made from them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagnosticsView {
    pub report: DiagnosticsReport,
    /// Scrubbed, ready to copy.
    pub text: String,
}

/// Scrub the free-text facts (error messages can carry an address with a token, or a path with the user's name) before
/// they reach the page, not only the copied text. Found in the real window: a version list refresh that failed named its
/// whole address, token included, in `registry.problem`.
pub fn scrub_facts(report: &mut DiagnosticsReport, user_home: Option<&Path>) {
    for service in &mut report.services {
        if let Some(error) = &service.error {
            service.error = Some(scrub(error, user_home));
        }
    }
    for job in &mut report.failed_jobs {
        job.error = scrub(&job.error, user_home);
        job.label = scrub(&job.label, user_home);
    }
    if let Some(problem) = &report.registry.problem {
        report.registry.problem = Some(scrub(problem, user_home));
    }
}

/// The user's home folder, which a report shortens to `~`.
pub fn user_home() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

// ---------------------------------------------------------------------------------------------
// The text and its scrubbing
// ---------------------------------------------------------------------------------------------

/// Words whose value is never written: `password=…`, `token: …`, `DB_PASSWORD=…`, `api_key=…`.
const SECRET_WORDS: [&str; 6] = ["password", "passwd", "secret", "token", "apikey", "api_key"];

/// Replace the value after a secret-looking word (`token=abc`, `password: abc`, `"secret":"abc"`) by `***`.
fn mask_secrets(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        let hit = SECRET_WORDS
            .iter()
            .find(|word| lower[i..].starts_with(*word));
        let Some(word) = hit else {
            let ch = text[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        };
        // The word itself (and any letters that continue it: `DB_PASSWORD`, `tokens`).
        let mut j = i + word.len();
        while j < text.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        out.push_str(&text[i..j]);
        // A separator (`=`, `:`, quotes, spaces) and then a value: mask the value.
        let mut k = j;
        while k < text.len() && matches!(bytes[k], b'=' | b':' | b'"' | b'\'' | b' ') {
            k += 1;
        }
        // Only `=` or `:` makes what follows a value: "no secrets here" is prose.
        let assigns = text[j..k].contains(['=', ':']);
        if assigns && k < text.len() && bytes[k..].first().is_some_and(|b| !b.is_ascii_whitespace())
        {
            out.push_str(&text[j..k]);
            out.push_str("***");
            let mut end = k;
            while end < text.len()
                && !matches!(
                    bytes[end],
                    b'&' | b' ' | b'\n' | b'\r' | b'"' | b'\'' | b',' | b';'
                )
            {
                end += 1;
            }
            i = end;
        } else {
            i = j;
        }
    }
    out
}

/// What a report says may not leak: the user's home folder (it holds their name) becomes `~`, in both slash styles and
/// whatever its case on Windows, and secret-looking values become `***`.
pub fn scrub(text: &str, user_home: Option<&Path>) -> String {
    let mut text = text.to_owned();
    if let Some(home) = user_home
        .map(|h| h.display().to_string())
        .filter(|h| h.len() > 3)
    {
        for form in [
            home.clone(),
            home.replace('\\', "/"),
            home.replace('/', "\\"),
        ] {
            let lower_text = text.to_lowercase();
            let lower_form = form.to_lowercase();
            let mut out = String::with_capacity(text.len());
            let mut last = 0;
            for (at, _) in lower_text.match_indices(&lower_form) {
                // Indices are the same in the lowercased text only for text whose lowercase has the same length.
                if lower_text.len() != text.len() {
                    break;
                }
                out.push_str(&text[last..at]);
                out.push('~');
                last = at + form.len();
            }
            out.push_str(&text[last..]);
            text = out;
        }
    }
    mask_secrets(&text)
}

/// The report as plain text, in English (maintainers read every report), scrubbed.
pub fn report_text(report: &DiagnosticsReport, user_home: Option<&Path>) -> String {
    let mut t = String::new();
    let a = &report.app;
    let _ = writeln!(t, "## lunaRun diagnostics");
    let _ = writeln!(t);
    let _ = writeln!(t, "- Version: {}", a.app_version);
    let _ = writeln!(t, "- System: {:?} {:?}", a.os, a.arch);
    let source = match report.home_source {
        HomeSource::Env => "from LUNA_HOME",
        HomeSource::Chosen => "chosen at setup",
        HomeSource::Default => "default",
    };
    let _ = writeln!(t, "- Data folder: {} ({source})", a.luna_home);
    match report.free_bytes {
        Some(free) => {
            let _ = writeln!(
                t,
                "- Free space there: {:.1} GB",
                free as f64 / 1_073_741_824.0
            );
        }
        None => {
            let _ = writeln!(t, "- Free space there: unknown");
        }
    }
    let vc = match report.vc_redist {
        VcRedistState::NotNeeded => None,
        VcRedistState::Ok => Some("installed"),
        VcRedistState::Missing => Some("MISSING"),
        VcRedistState::Outdated => Some("too old"),
        VcRedistState::Unknown => Some("could not be checked"),
    };
    if let Some(vc) = vc {
        let _ = writeln!(t, "- Visual C++ runtime: {vc}");
    }
    let _ = writeln!(t, "- Projects: {}", report.projects);
    let _ = writeln!(
        t,
        "- Installed: {}",
        if report.installed.is_empty() {
            "nothing".to_owned()
        } else {
            report.installed.join(", ")
        }
    );
    let r = &report.registry;
    let _ = writeln!(
        t,
        "- Version list: {:?}{}{}{}",
        r.source,
        r.age_seconds
            .map(|s| format!(", {} min old", s / 60))
            .unwrap_or_default(),
        r.custom_host
            .as_deref()
            .map(|h| format!(", custom address ({h})"))
            .unwrap_or_default(),
        r.problem
            .as_deref()
            .map(|p| format!(", problem: {p}"))
            .unwrap_or_default(),
    );
    if !report.changed_settings.is_empty() {
        let _ = writeln!(
            t,
            "- Settings changed: {}",
            report.changed_settings.join("; ")
        );
    }

    let _ = writeln!(t);
    let _ = writeln!(t, "### Ports");
    for p in &report.ports {
        let state = if !p.held {
            "free".to_owned()
        } else if p.by_lunarun {
            "used by lunaRun".to_owned()
        } else {
            format!(
                "TAKEN by {}",
                p.owner.as_deref().unwrap_or("another program")
            )
        };
        let what = p
            .assigned_to
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let _ = writeln!(t, "- {}{what}: {state}", p.port);
    }

    let _ = writeln!(t);
    let _ = writeln!(t, "### Services");
    if report.services.is_empty() {
        let _ = writeln!(t, "- none running");
    }
    for s in &report.services {
        let _ = writeln!(
            t,
            "- {}: {:?}{}",
            s.id,
            s.state,
            s.error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        );
    }
    if !report.failed_jobs.is_empty() {
        let _ = writeln!(t);
        let _ = writeln!(t, "### Failed tasks (this session)");
        for j in &report.failed_jobs {
            let _ = writeln!(t, "- {}: {}", j.label, j.error);
        }
    }
    scrub(&t, user_home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Arch, Os};
    use crate::error::{ErrorCode, LunaError};

    fn report() -> DiagnosticsReport {
        DiagnosticsReport {
            app: AppInfo {
                app_version: "0.0.2".to_owned(),
                os: Os::Windows,
                arch: Arch::X86_64,
                luna_home: r"C:\Users\Ada Lovelace\AppData\Local\lunaRun".to_owned(),
                luna_home_from_env: false,
            },
            home_source: HomeSource::Default,
            free_bytes: Some(40 * 1_073_741_824),
            vc_redist: VcRedistState::Ok,
            installed: vec!["php 8.6.0".to_owned(), "mysql 8.4.13".to_owned()],
            projects: 3,
            ports: vec![
                PortCheck {
                    port: 3306,
                    assigned_to: Some("mysql@8.4".to_owned()),
                    held: true,
                    by_lunarun: false,
                    owner: Some("mysqld.exe".to_owned()),
                },
                PortCheck {
                    port: 8101,
                    assigned_to: Some("site@4b1e0c".to_owned()),
                    held: false,
                    by_lunarun: false,
                    owner: None,
                },
            ],
            services: vec![ServiceLine {
                id: "mysql@8.4".to_owned(),
                state: ServiceState::Error,
                error: Some(r"PORT_IN_USE: port 3306 is used by C:\xampp\mysql\bin\mysqld.exe".to_owned()),
            }],
            failed_jobs: vec![FailedJob {
                label: "Install PHP 8.6.0".to_owned(),
                error: r"NETWORK: https://example.org/php.zip?token=s3cr3t-abc failed; saved in C:\Users\Ada Lovelace\Downloads".to_owned(),
            }],
            registry: RegistryLine {
                source: RegistrySourceKind::Cache,
                age_seconds: Some(3600),
                problem: None,
                custom_host: Some("mirror.example.org".to_owned()),
            },
            changed_settings: vec!["default PHP line = 8.4".to_owned()],
        }
    }

    #[test]
    fn the_report_says_what_a_maintainer_needs() {
        let text = report_text(&report(), Some(Path::new(r"C:\Users\Ada Lovelace")));
        for wanted in [
            "## lunaRun diagnostics",
            "- Version: 0.0.2",
            "- System: Windows X86_64",
            r"- Data folder: ~\AppData\Local\lunaRun (default)",
            "- Free space there: 40.0 GB",
            "- Visual C++ runtime: installed",
            "- Projects: 3",
            "- Installed: php 8.6.0, mysql 8.4.13",
            "custom address (mirror.example.org)",
            "- 3306 (mysql@8.4): TAKEN by mysqld.exe",
            "- 8101 (site@4b1e0c): free",
            "- mysql@8.4: Error (PORT_IN_USE",
            "### Failed tasks (this session)",
            "- Settings changed: default PHP line = 8.4",
        ] {
            assert!(text.contains(wanted), "missing {wanted:?} in:\n{text}");
        }
    }

    #[test]
    fn the_user_name_and_secrets_never_reach_the_report() {
        let text = report_text(&report(), Some(Path::new(r"C:\Users\Ada Lovelace")));
        assert!(!text.contains("Ada"), "the user name is in:\n{text}");
        assert!(!text.contains("s3cr3t"), "a token is in:\n{text}");
        assert!(text.contains("token=***"), "{text}");
        assert!(text.contains(r"~\Downloads"), "{text}");
    }

    #[test]
    fn secret_looking_values_are_masked_in_every_usual_shape() {
        for (input, leak) in [
            ("DB_PASSWORD=hunter2 next", "hunter2"),
            ("password: hunter2", "hunter2"),
            (r#"{"secret":"abc123"}"#, "abc123"),
            ("https://x.org/a?api_key=k-1&b=2", "k-1"),
            ("Authorization token=t0k3n", "t0k3n"),
            ("MAIL_PASSWORD='p@ss w'", "p@ss"),
        ] {
            let out = mask_secrets(input);
            assert!(!out.contains(leak), "{input:?} became {out:?}");
            assert!(out.contains("***"), "{input:?} became {out:?}");
        }
        // Words that only look alike, and text with no value, are left alone.
        assert_eq!(mask_secrets("no secrets here"), "no secrets here");
        assert_eq!(mask_secrets("tokens: none"), "tokens: ***");
        assert_eq!(mask_secrets("a password"), "a password");
    }

    #[test]
    fn the_home_folder_is_shortened_in_both_slash_styles_and_any_case() {
        let home = Path::new(r"C:\Users\José");
        let out = scrub(
            r"C:\Users\José\x and c:/users/josé/y and C:\USERS\JOSÉ\z",
            Some(home),
        );
        assert_eq!(out, r"~\x and ~/y and ~\z");
        // Nothing to shorten without a home.
        assert_eq!(scrub("C:/a", None), "C:/a");
    }

    #[test]
    fn the_facts_are_gathered_without_secrets_and_with_the_host_only() {
        let settings = Settings {
            default_node_line: Some("22".to_owned()),
            registry_url: Some("https://mirror.example.org/r.json?token=zzz".to_owned()),
            ..Settings::default()
        };
        let changed = changed_settings(&settings);
        assert_eq!(
            changed,
            [
                "default Node.js line = 22",
                "version list address = custom (mirror.example.org)"
            ]
        );
        assert!(changed_settings(&Settings::default()).is_empty());

        let status = RegistryStatus {
            source: RegistrySourceKind::Fresh,
            age_seconds: Some(5),
            problem: Some(LunaError::new(ErrorCode::Network, "offline")),
            refreshing: false,
        };
        let line = registry_line(&status, settings.registry_url.as_deref());
        assert_eq!(line.custom_host.as_deref(), Some("mirror.example.org"));
        assert_eq!(line.problem.as_deref(), Some("NETWORK: offline"));
    }

    #[test]
    fn the_facts_the_page_shows_are_scrubbed_too() {
        // A failed refresh names its whole address: the page must not show the token either.
        let mut facts = report();
        facts.registry.problem =
            Some("NETWORK: https://mirror.example.org/r.json?token=leak-1 answered 404".to_owned());
        facts.services[0].error = Some(r"IO: C:\Users\Ada Lovelace\x: password=leak-2".to_owned());
        scrub_facts(&mut facts, Some(Path::new(r"C:\Users\Ada Lovelace")));
        let json = serde_json::to_string(&facts).unwrap();
        for leak in ["leak-1", "leak-2", "s3cr3t", "Ada Lovelace\\\\x"] {
            assert!(!json.contains(leak), "{leak} in {json}");
        }
        assert!(json.contains("token=***"), "{json}");
    }

    #[test]
    fn a_port_held_by_lunaruns_own_program_is_told_apart() {
        let home = Path::new(r"C:\Users\x\AppData\Local\lunaRun");
        let ours = PortOwner {
            pid: 1,
            process_name: Some("caddy.exe".to_owned()),
            process_path: Some(
                r"C:\Users\x\AppData\Local\lunaRun\runtimes\caddy\2.11.9\caddy.exe".to_owned(),
            ),
        };
        let theirs = PortOwner {
            pid: 2,
            process_name: Some("httpd.exe".to_owned()),
            process_path: Some(r"C:\xampp\apache\bin\httpd.exe".to_owned()),
        };
        let mine = port_check(8101, Some("site@a".to_owned()), &[ours], home);
        assert!(mine.held && mine.by_lunarun);
        let other = port_check(80, None, &[theirs], home);
        assert!(other.held && !other.by_lunarun);
        assert_eq!(other.owner.as_deref(), Some("httpd.exe"));
        let free = port_check(443, None, &[], home);
        assert!(!free.held && !free.by_lunarun && free.owner.is_none());
    }
}
