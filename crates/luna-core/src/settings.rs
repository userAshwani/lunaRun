//! The user's settings (FR-AP-03; D-045): `config/settings.json`, with a `schema` (D-006).
//!
//! Only what the user changed is stored; everything else is the built-in default, so a new default reaches users who
//! never touched it. Every value is checked before it is saved (`INVALID_INPUT` with `details.field`), and the file is
//! written atomically. What each setting changes and when:
//!
//! - **Default PHP and Node.js line**: the line a project gets when it does not say (`ResolvePrefs`); at once, on the next
//!   read of a project.
//! - **Port ranges**: where new ports come from (`PortAllocator`); after a restart. A port already given to a service or a
//!   site stays, so nothing running moves.
//! - **Registry address**: where the list of versions is downloaded from (the `LUNA_REGISTRY_URL` environment variable
//!   still wins); on the next refresh. HTTPS only.
//!
//! Launch at login is not stored here: the operating system is the truth (a Run key, a LaunchAgent, an autostart file),
//! so the page asks it. The theme stays in the page (it must apply before the backend answers, with no flash).

use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::error::{ErrorCode, LunaError, Result};
use crate::paths::LunaHome;
use crate::services::PortRanges;
use crate::state::{self, Persisted};

/// The line a project gets when it does not say which it needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeDefaults {
    pub php_line: Option<String>,
    pub node_line: Option<String>,
}

impl RuntimeDefaults {
    /// No default: the newest line that fits is used (the behaviour before settings existed).
    pub const NONE: Self = Self {
        php_line: None,
        node_line: None,
    };
}

/// A range of ports, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortSpan {
    pub start: u16,
    pub end: u16,
}

impl PortSpan {
    fn range(self) -> RangeInclusive<u16> {
        self.start..=self.end
    }

    fn of(range: &RangeInclusive<u16>) -> Self {
        Self {
            start: *range.start(),
            end: *range.end(),
        }
    }

    fn len(self) -> u32 {
        u32::from(self.end) - u32::from(self.start) + 1
    }

    fn overlaps(self, other: Self) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// The four ranges of SERVICES section 3, as the page shows and edits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortSettings {
    pub site: PortSpan,
    pub php_fastcgi: PortSpan,
    pub database: PortSpan,
    /// One port.
    pub caddy_admin: u16,
}

impl PortSettings {
    pub fn defaults() -> Self {
        let d = PortRanges::default();
        Self {
            site: PortSpan::of(&d.site),
            php_fastcgi: PortSpan::of(&d.php_fastcgi),
            database: PortSpan::of(&d.database),
            caddy_admin: *d.caddy_admin.start(),
        }
    }

    pub fn ranges(self) -> PortRanges {
        PortRanges {
            site: self.site.range(),
            php_fastcgi: self.php_fastcgi.range(),
            database: self.database.range(),
            caddy_admin: self.caddy_admin..=self.caddy_admin,
        }
    }
}

/// The settings as the page shows them: every value filled in (stored or default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    /// `8.4`, or none for "the newest that fits".
    pub default_php_line: Option<String>,
    /// `22`, or none for "the newest LTS that fits".
    pub default_node_line: Option<String>,
    pub ports: PortSettings,
    /// None for the built-in address.
    pub registry_url: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_php_line: None,
            default_node_line: None,
            ports: PortSettings::defaults(),
            registry_url: None,
        }
    }
}

impl Settings {
    pub fn runtime_defaults(&self) -> RuntimeDefaults {
        RuntimeDefaults {
            php_line: self.default_php_line.clone(),
            node_line: self.default_node_line.clone(),
        }
    }
}

/// What the Settings page shows: the settings, the defaults (for "reset" and for the hints), and the data folder (which is
/// only shown: it is chosen in the first-run wizard, D-044).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsView {
    pub current: Settings,
    pub defaults: Settings,
    pub home: String,
    pub home_source: crate::onboarding::HomeSource,
    /// The built-in address of the version list.
    pub default_registry_url: String,
}

/// The page's view of the settings.
///
/// # Errors
/// As [`load`].
pub fn view(home: &LunaHome) -> Result<SettingsView> {
    Ok(SettingsView {
        current: load(home)?,
        defaults: Settings::default(),
        home: home.root().display().to_string(),
        home_source: if home.from_env() {
            crate::onboarding::HomeSource::Env
        } else if home.is_chosen() {
            crate::onboarding::HomeSource::Chosen
        } else {
            crate::onboarding::HomeSource::Default
        },
        default_registry_url: crate::registry::DEFAULT_REGISTRY_URL.to_owned(),
    })
}

/// What is stored: only what differs from the defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_php_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_node_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ports: Option<PortSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registry_url: Option<String>,
}

impl Persisted for SettingsFile {
    const SCHEMA_VERSION: u32 = 1;
}

pub fn settings_path(home: &LunaHome) -> PathBuf {
    home.config_dir().join("settings.json")
}

fn invalid(field: &str, message: String) -> LunaError {
    LunaError::new(ErrorCode::InvalidInput, message).with_details(json!({ "field": field }))
}

/// `8.4` for PHP, `22` for Node.js: the shape of a line, nothing else (the page offers the registry's lines).
fn check_line(field: &str, value: Option<&str>, parts: usize) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let pieces: Vec<&str> = value.split('.').collect();
    let ok = pieces.len() == parts
        && pieces
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err(invalid(field, format!("{value:?} is not a version line")))
    }
}

/// Smallest sizes that still let lunaRun work: a site per project, a Windows pool of four per PHP version, a database
/// line or two.
const MIN_SITE: u32 = 10;
const MIN_PHP: u32 = 8;
const MIN_DATABASE: u32 = 2;
/// Below this a port is privileged on macOS and Linux and often taken on Windows.
const LOWEST: u16 = 1024;

fn check_ports(ports: &PortSettings) -> Result<()> {
    let spans = [
        ("ports.site", ports.site, MIN_SITE),
        ("ports.phpFastcgi", ports.php_fastcgi, MIN_PHP),
        ("ports.database", ports.database, MIN_DATABASE),
        (
            "ports.caddyAdmin",
            PortSpan {
                start: ports.caddy_admin,
                end: ports.caddy_admin,
            },
            1,
        ),
    ];
    for (field, span, minimum) in spans {
        if span.start < LOWEST {
            return Err(invalid(
                field,
                format!(
                    "ports below {LOWEST} are reserved by the system: {}",
                    span.start
                ),
            ));
        }
        if span.start > span.end {
            return Err(invalid(
                field,
                format!(
                    "the range starts after it ends: {} to {}",
                    span.start, span.end
                ),
            ));
        }
        if span.len() < minimum {
            return Err(invalid(
                field,
                format!(
                    "the range needs at least {minimum} ports, it has {}",
                    span.len()
                ),
            ));
        }
    }
    for (i, (field, span, _)) in spans.iter().enumerate() {
        for (other_field, other, _) in &spans[i + 1..] {
            if span.overlaps(*other) {
                return Err(invalid(
                    field,
                    format!("{field} and {other_field} overlap: a port can only have one use"),
                ));
            }
        }
    }
    Ok(())
}

/// Only HTTPS (the list decides what is downloaded and run; its checksums come from it too).
fn check_url(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    match reqwest::Url::parse(value) {
        Ok(url) if url.scheme() == "https" && url.host_str().is_some() => Ok(()),
        _ => Err(LunaError::new(
            ErrorCode::InvalidUrl,
            format!("{value:?} is not an https:// address"),
        )
        .with_details(json!({ "field": "registryUrl" }))),
    }
}

/// Check every value of `settings`.
///
/// # Errors
/// `INVALID_INPUT` (`details.field` names the value) or `INVALID_URL` for the registry address.
pub fn validate(settings: &Settings) -> Result<()> {
    check_line("defaultPhpLine", settings.default_php_line.as_deref(), 2)?;
    check_line("defaultNodeLine", settings.default_node_line.as_deref(), 1)?;
    check_ports(&settings.ports)?;
    check_url(settings.registry_url.as_deref())
}

/// The settings, with the defaults for whatever was never changed. A damaged file is moved aside and the defaults are
/// used (`state::load`).
///
/// # Errors
/// I/O errors and a file written by a newer lunaRun.
pub fn load(home: &LunaHome) -> Result<Settings> {
    let file = state::load::<SettingsFile>(&settings_path(home))?.value;
    let defaults = Settings::default();
    let settings = Settings {
        default_php_line: file.default_php_line,
        default_node_line: file.default_node_line,
        ports: file.ports.unwrap_or(defaults.ports),
        registry_url: file.registry_url,
    };
    // A file edited by hand into something lunaRun would refuse counts as not set.
    Ok(if validate(&settings).is_ok() {
        settings
    } else {
        defaults
    })
}

/// Check and store `settings`. A value equal to the default is not stored, so a later default reaches it.
///
/// # Errors
/// As [`validate`], and I/O errors. Nothing is written when a value is refused.
pub fn save(home: &LunaHome, settings: &Settings) -> Result<Settings> {
    let mut settings = settings.clone();
    // Spaces around a typed value are not part of it; an empty field means the default.
    let tidy = |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    settings.default_php_line = tidy(settings.default_php_line);
    settings.default_node_line = tidy(settings.default_node_line);
    settings.registry_url = tidy(settings.registry_url);
    validate(&settings)?;
    let defaults = PortSettings::defaults();
    let file = SettingsFile {
        default_php_line: settings.default_php_line.clone(),
        default_node_line: settings.default_node_line.clone(),
        ports: (settings.ports != defaults).then_some(settings.ports),
        registry_url: settings.registry_url.clone(),
    };
    state::save(&settings_path(home), &file)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, LunaHome) {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        home.ensure_layout().unwrap();
        (tmp, home)
    }

    #[test]
    fn with_nothing_stored_everything_is_the_default() {
        let (_tmp, home) = home();
        let settings = load(&home).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(
            settings.ports.site,
            PortSpan {
                start: 8100,
                end: 8999
            }
        );
        assert_eq!(settings.ports.caddy_admin, 2119);
        assert_eq!(settings.runtime_defaults(), RuntimeDefaults::NONE);
    }

    #[test]
    fn a_change_is_stored_and_read_back_and_only_what_differs_is_in_the_file() {
        let (_tmp, home) = home();
        let settings = Settings {
            default_php_line: Some(" 8.4 ".to_owned()),
            default_node_line: Some("22".to_owned()),
            ..Settings::default()
        };
        let saved = save(&home, &settings).unwrap();
        assert_eq!(saved.default_php_line.as_deref(), Some("8.4"), "trimmed");

        let again = load(&home).unwrap();
        assert_eq!(again, saved);
        let text = std::fs::read_to_string(settings_path(&home)).unwrap();
        assert!(text.contains("\"defaultPhpLine\": \"8.4\""), "{text}");
        assert!(
            !text.contains("ports"),
            "the default ports are not stored: {text}"
        );
        assert!(!text.contains("registryUrl"), "{text}");
        assert!(text.contains("\"schema\": 1"), "{text}");
    }

    #[test]
    fn an_empty_field_goes_back_to_the_default() {
        let (_tmp, home) = home();
        let mut settings = Settings {
            default_php_line: Some("8.4".to_owned()),
            ..Settings::default()
        };
        save(&home, &settings).unwrap();
        settings.default_php_line = Some("  ".to_owned());
        save(&home, &settings).unwrap();
        assert_eq!(load(&home).unwrap().default_php_line, None);
    }

    #[test]
    fn a_line_must_look_like_one() {
        for (php, ok) in [
            ("8.4", true),
            ("10.0", true),
            ("8", false),
            ("8.4.1", false),
            ("8.x", false),
            ("latest", false),
        ] {
            let settings = Settings {
                default_php_line: Some(php.to_owned()),
                ..Settings::default()
            };
            assert_eq!(validate(&settings).is_ok(), ok, "{php}");
        }
        for (node, ok) in [("22", true), ("8", true), ("22.1", false), ("lts", false)] {
            let settings = Settings {
                default_node_line: Some(node.to_owned()),
                ..Settings::default()
            };
            let result = validate(&settings);
            assert_eq!(result.is_ok(), ok, "{node}");
            if let Err(err) = result {
                assert_eq!(err.details.unwrap()["field"], "defaultNodeLine");
            }
        }
    }

    #[test]
    fn port_ranges_are_refused_when_they_are_too_small_reversed_privileged_or_overlapping() {
        let field_of = |ports: PortSettings| {
            let settings = Settings {
                ports,
                ..Settings::default()
            };
            validate(&settings)
                .err()
                .map(|e| e.details.unwrap()["field"].as_str().unwrap().to_owned())
        };
        let d = PortSettings::defaults();
        assert_eq!(field_of(d), None);
        let span = |start, end| PortSpan { start, end };
        assert_eq!(
            field_of(PortSettings {
                site: span(8100, 8105),
                ..d
            })
            .as_deref(),
            Some("ports.site"),
            "fewer than ten"
        );
        assert_eq!(
            field_of(PortSettings {
                php_fastcgi: span(9200, 9100),
                ..d
            })
            .as_deref(),
            Some("ports.phpFastcgi"),
            "reversed"
        );
        assert_eq!(
            field_of(PortSettings {
                database: span(80, 90),
                ..d
            })
            .as_deref(),
            Some("ports.database"),
            "privileged"
        );
        assert_eq!(
            field_of(PortSettings {
                caddy_admin: 8500,
                ..d
            })
            .as_deref(),
            Some("ports.site"),
            "inside the site range"
        );
        assert_eq!(
            field_of(PortSettings {
                site: span(9000, 9150),
                ..d
            })
            .as_deref(),
            Some("ports.site"),
            "overlaps PHP"
        );
        // A range that moved and fits is fine.
        assert_eq!(
            field_of(PortSettings {
                site: span(18100, 18999),
                ..d
            }),
            None
        );
    }

    #[test]
    fn the_registry_address_must_be_https() {
        for (url, ok) in [
            ("https://example.org/registry.json", true),
            ("http://example.org/registry.json", false),
            ("file:///C:/registry.json", false),
            ("not a url", false),
            ("https://", false),
        ] {
            let settings = Settings {
                registry_url: Some(url.to_owned()),
                ..Settings::default()
            };
            let result = validate(&settings);
            assert_eq!(result.is_ok(), ok, "{url}");
            if let Err(err) = result {
                assert_eq!(err.code, ErrorCode::InvalidUrl);
                assert_eq!(err.details.unwrap()["field"], "registryUrl");
            }
        }
    }

    #[test]
    fn a_refused_value_writes_nothing_and_a_file_edited_into_nonsense_is_ignored() {
        let (_tmp, home) = home();
        let bad = Settings {
            registry_url: Some("http://plain.example".to_owned()),
            ..Settings::default()
        };
        assert!(save(&home, &bad).is_err());
        assert!(!settings_path(&home).exists());

        std::fs::write(
            settings_path(&home),
            r#"{"schema":1,"defaultPhpLine":"eight","registryUrl":"https://ok.example/r.json"}"#,
        )
        .unwrap();
        assert_eq!(load(&home).unwrap(), Settings::default());
    }
}
