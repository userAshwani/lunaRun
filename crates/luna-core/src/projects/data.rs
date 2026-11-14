//! The tables detection reads (FR-PJ-07): which PHP each Laravel major supports, what lunaRun's PHP has of each extension,
//! which Node major numbers are long-term-support lines. They are JSON files in `crates/luna-core/data/`, built into the
//! app, so a new Laravel or a new LTS line is a row in a file, not a change of code.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::Availability;
use crate::constraints::{VersionReq, parse_composer};
use crate::error::{LunaError, Result};

const LARAVEL_PHP: &str = include_str!("../../data/laravel-php.json");
const PHP_EXTENSIONS: &str = include_str!("../../data/php-extensions.json");
const NODE_LTS: &str = include_str!("../../data/node-lts.json");

/// The only schema these files are read in.
const SCHEMA: u32 = 1;

#[derive(Deserialize)]
struct RawLaravel {
    schema: u32,
    laravel: Vec<RawLaravelRow>,
}

#[derive(Deserialize)]
struct RawLaravelRow {
    framework: String,
    php: String,
}

#[derive(Deserialize)]
struct RawExtensions {
    schema: u32,
    extensions: BTreeMap<String, RawExtension>,
    #[serde(rename = "laravelBaseline")]
    laravel_baseline: Vec<String>,
}

#[derive(Deserialize)]
struct RawExtension {
    ini: Option<String>,
    #[serde(default)]
    zend: bool,
    windows: String,
    unix: String,
}

#[derive(Deserialize)]
struct RawNode {
    schema: u32,
    codenames: BTreeMap<String, u64>,
}

/// One row of the Laravel to PHP table: the Laravel versions it covers and the PHP versions they support.
#[derive(Debug, Clone)]
pub(super) struct LaravelRow {
    pub framework: VersionReq,
    pub framework_text: String,
    pub php: VersionReq,
    pub php_text: String,
}

/// What lunaRun's PHP has of one extension.
#[derive(Debug, Clone)]
pub(super) struct ExtensionInfo {
    pub ini: Option<String>,
    pub zend: bool,
    pub windows: Availability,
    pub unix: Availability,
}

/// The tables [`detect`](super::detect) and [`resolve`](super::resolve) read.
#[derive(Debug, Clone)]
pub struct DetectionData {
    pub(super) laravel: Vec<LaravelRow>,
    pub(super) extensions: BTreeMap<String, ExtensionInfo>,
    pub(super) laravel_baseline: Vec<String>,
    /// Lower-case codename to major number.
    pub(super) node_lts: BTreeMap<String, u64>,
}

fn invalid(file: &str, what: impl std::fmt::Display) -> LunaError {
    LunaError::internal(format!("detection data {file}: {what}"))
}

fn availability(file: &str, name: &str, text: &str) -> Result<Availability> {
    match text {
        "enabled" => Ok(Availability::Enabled),
        "disabled" => Ok(Availability::Disabled),
        "absent" => Ok(Availability::Absent),
        other => Err(invalid(
            file,
            format!("{name}: '{other}' is not enabled, disabled or absent"),
        )),
    }
}

impl DetectionData {
    /// The tables that are built into the app.
    ///
    /// # Errors
    /// `INTERNAL` if a built-in file is damaged. A test reads them, so this is a bug found before release.
    pub fn bundled() -> Result<Self> {
        Self::from_json(LARAVEL_PHP, PHP_EXTENSIONS, NODE_LTS)
    }

    /// Read the three tables from their JSON text.
    ///
    /// # Errors
    /// `INTERNAL` for text that is not JSON of the expected shape, a schema other than 1, a constraint that cannot be
    /// read, or a value that is not `enabled`, `disabled` or `absent`.
    pub fn from_json(laravel: &str, extensions: &str, node: &str) -> Result<Self> {
        let laravel: RawLaravel =
            serde_json::from_str(laravel).map_err(|e| invalid("laravel-php.json", e))?;
        let extensions: RawExtensions =
            serde_json::from_str(extensions).map_err(|e| invalid("php-extensions.json", e))?;
        let node: RawNode = serde_json::from_str(node).map_err(|e| invalid("node-lts.json", e))?;
        for (file, schema) in [
            ("laravel-php.json", laravel.schema),
            ("php-extensions.json", extensions.schema),
            ("node-lts.json", node.schema),
        ] {
            if schema != SCHEMA {
                return Err(invalid(
                    file,
                    format!("schema {schema}, this lunaRun reads schema {SCHEMA}"),
                ));
            }
        }

        let read = |file: &str, text: &str| {
            parse_composer(text)
                .map(|parsed| parsed.req)
                .map_err(|e| invalid(file, format!("'{text}': {e}")))
        };
        let mut rows = Vec::new();
        for row in laravel.laravel {
            rows.push(LaravelRow {
                framework: read("laravel-php.json", &row.framework)?,
                php: read("laravel-php.json", &row.php)?,
                framework_text: row.framework,
                php_text: row.php,
            });
        }

        let mut table = BTreeMap::new();
        for (name, raw) in extensions.extensions {
            let info = ExtensionInfo {
                windows: availability("php-extensions.json", &name, &raw.windows)?,
                unix: availability("php-extensions.json", &name, &raw.unix)?,
                ini: raw.ini,
                zend: raw.zend,
            };
            table.insert(name.to_ascii_lowercase(), info);
        }
        let node_lts = node
            .codenames
            .into_iter()
            .map(|(name, major)| (name.to_ascii_lowercase(), major))
            .collect();
        Ok(Self {
            laravel: rows,
            extensions: table,
            laravel_baseline: extensions.laravel_baseline,
            node_lts,
        })
    }

    /// The major numbers of the long-term-support Node lines in the table, newest first.
    pub fn lts_majors(&self) -> Vec<u64> {
        let mut majors: Vec<u64> = self.node_lts.values().copied().collect();
        majors.sort_unstable_by(|a, b| b.cmp(a));
        majors.dedup();
        majors
    }

    /// The Node major a codename (`iron`, `Jod`) belongs to.
    pub fn lts_major_of(&self, codename: &str) -> Option<u64> {
        self.node_lts.get(&codename.to_ascii_lowercase()).copied()
    }

    /// Is `major` a long-term-support line?
    pub fn is_lts_major(&self, major: u64) -> bool {
        self.node_lts.values().any(|m| *m == major)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RuntimeVersion;
    use crate::runtimes::WINDOWS_EXTENSIONS;

    fn v(text: &str) -> RuntimeVersion {
        RuntimeVersion::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn the_built_in_tables_are_readable_and_say_when_they_were_checked() {
        let data = DetectionData::bundled().unwrap();
        assert!(data.laravel.len() >= 8, "Laravel 6 to 13");
        assert!(data.extensions.len() > 60);
        for file in [LARAVEL_PHP, PHP_EXTENSIONS, NODE_LTS] {
            let value: serde_json::Value = serde_json::from_str(file).unwrap();
            assert!(
                value["checked"].as_str().is_some_and(|d| d.len() == 10),
                "a date it was checked"
            );
            assert!(
                value["source"].as_str().is_some_and(|s| s.starts_with("")),
                "and where from"
            );
            assert!(!value["source"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn each_laravel_major_has_one_row_that_covers_its_versions_and_no_other_major() {
        let data = DetectionData::bundled().unwrap();
        for major in 6..=13 {
            let version = v(&format!("{major}.5.3"));
            let rows: Vec<&LaravelRow> = data
                .laravel
                .iter()
                .filter(|r| r.framework.matches(&version))
                .collect();
            assert_eq!(rows.len(), 1, "Laravel {major}.5.3 has exactly one row");
        }
        assert!(
            data.laravel.iter().all(|r| !r.php.is_empty()),
            "every row allows some PHP"
        );
        let ten = data
            .laravel
            .iter()
            .find(|r| r.framework.matches(&v("10.48.4")))
            .unwrap();
        assert_eq!(
            ten.php.to_string(),
            ">=8.1.0 <8.4.0",
            "Laravel 10: PHP 8.1 to 8.3"
        );
    }

    #[test]
    fn the_extension_table_agrees_with_what_lunarun_switches_on() {
        let data = DetectionData::bundled().unwrap();
        for name in WINDOWS_EXTENSIONS {
            let info = data
                .extensions
                .get(name)
                .unwrap_or_else(|| panic!("{name} is switched on on Windows but not in the table"));
            assert_eq!(info.windows, Availability::Enabled, "{name}");
        }
        for name in &data.laravel_baseline {
            assert!(
                data.extensions.contains_key(name),
                "the Laravel baseline names {name}, which the table does not know"
            );
            let info = &data.extensions[name];
            assert_eq!(info.windows, Availability::Enabled, "{name}");
            assert_eq!(
                info.unix,
                Availability::Enabled,
                "{name}: Laravel does not run without it"
            );
        }
        // The two builds really differ, and the table says so.
        assert_eq!(data.extensions["intl"].unix, Availability::Absent);
        assert_eq!(data.extensions["intl"].windows, Availability::Enabled);
        assert!(data.extensions["zend-opcache"].zend);
        assert_eq!(
            data.extensions["zend-opcache"].ini.as_deref(),
            Some("opcache")
        );
    }

    #[test]
    fn node_lts_lines_come_from_the_codenames() {
        let data = DetectionData::bundled().unwrap();
        assert_eq!(data.lts_major_of("Iron"), Some(20));
        assert_eq!(data.lts_major_of("jod"), Some(22));
        assert_eq!(data.lts_major_of("nonsense"), None);
        assert!(data.is_lts_major(24) && !data.is_lts_major(25) && !data.is_lts_major(23));
        let majors = data.lts_majors();
        assert_eq!(majors.first(), Some(&26));
        assert!(
            majors.windows(2).all(|w| w[0] > w[1]),
            "newest first, no duplicates"
        );
    }

    #[test]
    fn damaged_tables_are_refused_with_the_file_named() {
        let good = (LARAVEL_PHP, PHP_EXTENSIONS, NODE_LTS);
        let broken = |laravel: &str, extensions: &str, node: &str| {
            DetectionData::from_json(laravel, extensions, node)
                .unwrap_err()
                .message
        };
        assert!(broken("not json", good.1, good.2).contains("laravel-php.json"));
        assert!(broken(good.0, "{}", good.2).contains("php-extensions.json"));
        assert!(broken(good.0, good.1, "[]").contains("node-lts.json"));
        let newer = good.0.replace("\"schema\": 1", "\"schema\": 2");
        assert!(broken(&newer, good.1, good.2).contains("schema 2"));
        let bad_constraint = good.0.replace("^6.0", "^6.*");
        assert!(broken(&bad_constraint, good.1, good.2).contains("^6.*"));
        let bad_availability = good.1.replacen("\"enabled\"", "\"maybe\"", 1);
        assert!(broken(good.0, &bad_availability, good.2).contains("maybe"));
    }
}
