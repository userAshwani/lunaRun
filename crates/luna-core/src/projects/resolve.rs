//! `resolve`: from what a project needs to the version to use (docs/PROJECT-DETECTION.md section 7; D-035).
//!
//! Pure: it is given what is installed, what the registry offers for this computer and the user's preferences, and it
//! answers. Nothing here touches the disk or the network.

use super::data::DetectionData;
use super::detect::version_pin;
use super::{
    Basis, Binding, BindingMode, DatabaseSuggestion, DbEngine, DbNote, DetectionReport,
    ExtensionStatus, IssueCode, Recommendation, Requirement, Resolution, ResolutionIssue,
    RuntimeResolution, Selector, SourceKind,
};
use crate::constraints::VersionReq;
use crate::diagnostics::Os;
use crate::registry::{AvailableVersion, Channel, RuntimeCatalog, RuntimeVersion};
use crate::runtimes::InstalledRuntime;

/// The MySQL line suggested for a project that uses MySQL (SERVICES: the first long-term-support line).
const DEFAULT_MYSQL_LINE: &str = "8.4";

/// What the user prefers, and what they chose for this project.
#[derive(Debug, Clone)]
pub struct ResolvePrefs {
    /// The operating system the answer is for: what can be installed, and which build the extensions are in.
    pub os: Os,
    /// The PHP line to use when a project does not say (`8.3`).
    pub default_php_line: Option<String>,
    /// The Node line to use when a project does not say (`22`).
    pub default_node_line: Option<String>,
    /// A PHP version chosen for this project: a line (`8.3`) or a version (`8.3.14`). It wins over everything.
    pub php_override: Option<String>,
    pub node_override: Option<String>,
}

impl ResolvePrefs {
    /// No preferences, for this computer.
    pub fn new(os: Os) -> Self {
        Self {
            os,
            default_php_line: None,
            default_node_line: None,
            php_override: None,
            node_override: None,
        }
    }
}

/// A version that could be used: installed, or available to install.
#[derive(Debug, Clone)]
struct Candidate {
    version: RuntimeVersion,
    line: String,
    installed: bool,
    eol: bool,
    rosetta_fallback: bool,
}

/// The line of a version: `8.3` for PHP, `22` for Node.
fn line_of(runtime: &str, version: &RuntimeVersion) -> String {
    let numbers = version.numbers();
    if runtime == "node" {
        return numbers.first().map(u64::to_string).unwrap_or_default();
    }
    version.major_minor()
}

fn rows<'a>(available: &'a [RuntimeCatalog], runtime: &str) -> &'a [AvailableVersion] {
    available
        .iter()
        .find(|catalog| catalog.id == runtime)
        .map_or(&[], |catalog| catalog.versions.as_slice())
}

/// Installed versions of `runtime`, newest first, with their line and whether it is past its end of life.
fn installed_candidates(
    runtime: &str,
    installed: &[InstalledRuntime],
    catalog: &[AvailableVersion],
) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = installed
        .iter()
        .filter(|r| r.runtime == runtime)
        .filter_map(|r| RuntimeVersion::parse(&r.version))
        .map(|version| {
            let line = line_of(runtime, &version);
            let eol = catalog.iter().any(|row| row.line == line && row.eol);
            Candidate {
                version,
                line,
                installed: true,
                eol,
                rosetta_fallback: false,
            }
        })
        .collect();
    found.sort_by(|a, b| b.version.cmp(&a.version));
    found
}

/// Versions the registry has a download for on this computer, newest first. Pre-releases only when asked for.
fn available_candidates(
    runtime: &str,
    catalog: &[AvailableVersion],
    prereleases: bool,
) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = catalog
        .iter()
        .filter(|row| row.available && (prereleases || row.channel == Channel::Stable))
        .filter_map(|row| {
            RuntimeVersion::parse(&row.version).map(|version| Candidate {
                line: line_of(runtime, &version),
                version,
                installed: false,
                eol: row.eol,
                rosetta_fallback: row.rosetta_fallback,
            })
        })
        .collect();
    found.sort_by(|a, b| b.version.cmp(&a.version));
    found
}

/// What the requirement of a runtime comes to: its constraint, how to choose among the versions that satisfy it.
struct Spec {
    basis: Basis,
    req: VersionReq,
    text: Option<String>,
    selector: Selector,
    /// The version line to favour when it fits (`config.platform.php`).
    preferred_line: Option<String>,
    /// The binding is to this exact version, not to its line.
    exact: bool,
}

fn spec_for(requirement: Option<&Requirement>, override_text: Option<&str>) -> Spec {
    if let Some((req, exact)) = override_text.and_then(version_pin) {
        return Spec {
            basis: Basis::Manual,
            text: Some(req.to_string()),
            req,
            selector: Selector::Constraint,
            preferred_line: None,
            exact,
        };
    }
    let Some(requirement) = requirement else {
        return Spec {
            basis: Basis::NoRequirement,
            req: VersionReq::any(),
            text: None,
            selector: Selector::Constraint,
            preferred_line: None,
            exact: false,
        };
    };
    let basis = if requirement.pin.is_some() {
        Basis::Pinned
    } else if !requirement.sources.is_empty()
        && requirement
            .sources
            .iter()
            .all(|s| s.kind == SourceKind::FrameworkTable)
    {
        Basis::FrameworkTable
    } else {
        Basis::Constraint
    };
    Spec {
        basis,
        req: requirement.req.clone(),
        text: Some(requirement.constraint.clone()),
        selector: requirement.selector,
        preferred_line: if basis == Basis::Pinned {
            None
        } else {
            requirement.preferred.as_ref().map(|p| p.line.clone())
        },
        exact: requirement.pin.as_ref().is_some_and(|p| p.exact),
    }
}

/// Keep only the candidates on `line`, unless there are none: a preference, not a requirement.
fn favour_line(candidates: Vec<Candidate>, line: Option<&str>) -> Vec<Candidate> {
    let Some(line) = line else { return candidates };
    if candidates.iter().any(|c| c.line == line) {
        candidates.into_iter().filter(|c| c.line == line).collect()
    } else {
        candidates
    }
}

/// The line a team targets (`config.platform.php`), for installed and installable versions **together**: when anything
/// on that line fits, both lists are cut to it, so a project that targets 8.2 is offered an install of 8.2 rather than
/// quietly run on an 8.3 that happens to be installed. When nothing on the line fits the requirement, nothing changes.
fn target_line(
    installed: Vec<Candidate>,
    available: Vec<Candidate>,
    line: &str,
) -> (Vec<Candidate>, Vec<Candidate>) {
    let on_line = |c: &Candidate| c.line == line;
    if installed.iter().chain(available.iter()).any(on_line) {
        (
            installed.into_iter().filter(on_line).collect(),
            available.into_iter().filter(on_line).collect(),
        )
    } else {
        (installed, available)
    }
}

fn recommendation(candidate: &Candidate, exact: bool) -> Recommendation {
    Recommendation {
        version: candidate.version.as_str().to_owned(),
        line: candidate.line.clone(),
        installed: candidate.installed,
        needs_install: !candidate.installed,
        eol: candidate.eol,
        rosetta_fallback: candidate.rosetta_fallback,
        binding: if exact {
            Binding {
                mode: BindingMode::Exact,
                value: candidate.version.as_str().to_owned(),
            }
        } else {
            Binding {
                mode: BindingMode::Line,
                value: candidate.line.clone(),
            }
        },
    }
}

/// The name people use for a runtime in a sentence.
fn display(runtime: &str) -> &'static str {
    match runtime {
        "php" => "PHP",
        "node" => "Node.js",
        "mysql" => "MySQL",
        "mariadb" => "MariaDB",
        _ => "this runtime",
    }
}

fn os_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "Windows",
        Os::Macos => "macOS",
        Os::Linux => "Linux",
        Os::Unknown => "this operating system",
    }
}

/// What the answer is chosen from: the same for PHP and for Node.
struct World<'a> {
    installed: &'a [InstalledRuntime],
    available: &'a [RuntimeCatalog],
    data: &'a DetectionData,
    os: Os,
}

#[allow(
    clippy::too_many_lines,
    reason = "one pass: candidates, the choice, and the explanation when there is none"
)]
fn resolve_runtime(
    world: &World<'_>,
    runtime: &str,
    requirement: Option<&Requirement>,
    override_text: Option<&str>,
    default_line: Option<&str>,
) -> RuntimeResolution {
    let World {
        installed,
        available,
        data,
        os,
    } = *world;
    let catalog = rows(available, runtime);
    let spec = spec_for(requirement, override_text);
    let node = runtime == "node";

    // Which versions are in play: those that satisfy the requirement, and for `lts/*` those on a long-term-support line.
    let in_play = |c: &Candidate| -> bool {
        let major = c.version.numbers().first().copied().unwrap_or(0);
        match spec.selector {
            Selector::Constraint => {
                spec.req.matches(&c.version)
                    && (spec.basis != Basis::NoRequirement || !node || data.is_lts_major(major))
            }
            Selector::LatestLts => data.is_lts_major(major) && !c.version.is_prerelease(),
            Selector::Latest => !c.version.is_prerelease(),
        }
    };
    let prereleases = spec.req.allows_prereleases();
    let mut installed_c: Vec<Candidate> = installed_candidates(runtime, installed, catalog)
        .into_iter()
        .filter(&in_play)
        .collect();
    let mut available_c: Vec<Candidate> = available_candidates(runtime, catalog, prereleases)
        .into_iter()
        .filter(&in_play)
        .collect();
    let installed_matches: Vec<String> = installed_c
        .iter()
        .map(|c| c.version.as_str().to_owned())
        .collect();

    // Without a requirement the user's default line comes first, else the newest.
    let line_wanted = match spec.basis {
        Basis::NoRequirement => default_line.map(str::to_owned),
        _ => spec.preferred_line.clone(),
    };
    match (spec.basis, line_wanted.as_deref()) {
        (Basis::NoRequirement, line) => {
            installed_c = favour_line(installed_c, line);
            available_c = favour_line(available_c, line);
        }
        (_, Some(line)) => (installed_c, available_c) = target_line(installed_c, available_c, line),
        (_, None) => {}
    }

    // Installed first (the newest on a line that is still supported), else what could be installed.
    let pick = |list: &[Candidate]| {
        list.iter()
            .find(|c| !c.eol)
            .or_else(|| list.first())
            .cloned()
    };
    let chosen = pick(&installed_c).or_else(|| {
        // For something to install, the user's default line wins when it fits and is supported.
        let favoured = default_line.and_then(|line| {
            available_c
                .iter()
                .find(|c| c.line == line && !c.eol)
                .cloned()
        });
        favoured.or_else(|| pick(&available_c))
    });
    let recommended = chosen.as_ref().map(|c| recommendation(c, spec.exact));
    // A choice made by hand is allowed to disagree with the project (FR-PJ-04), and the page says when it does.
    let conflicts_with = match (spec.basis, requirement, chosen.as_ref()) {
        (Basis::Manual, Some(asked), Some(c)) if !asked.req.matches(&c.version) => {
            Some(asked.constraint.clone())
        }
        _ => None,
    };

    let mut issues = Vec::new();
    if recommended.is_none() {
        let wanted = spec
            .text
            .clone()
            .unwrap_or_else(|| "a default version".to_owned());
        if spec.selector == Selector::Constraint && spec.req.is_empty() {
            issues.push(ResolutionIssue {
                code: IssueCode::UnsatisfiableConstraint,
                message: format!(
                    "{wanted} cannot be satisfied by any {} version",
                    display(runtime)
                ),
                unavailable_lines: Vec::new(),
            });
        } else {
            let mut lines: Vec<String> = catalog
                .iter()
                .filter(|row| !row.available)
                .filter_map(|row| RuntimeVersion::parse(&row.version).map(|v| (row, v)))
                .filter(|(_, v)| spec.req.matches(v) || spec.req.allows_prereleases())
                .filter(|(_, v)| spec.selector != Selector::Constraint || spec.req.matches(v))
                .map(|(row, _)| row.line.clone())
                .collect();
            lines.sort();
            lines.dedup();
            let message = if lines.is_empty() {
                format!(
                    "no {} version known to lunaRun satisfies {wanted}",
                    display(runtime)
                )
            } else {
                format!(
                    "{} {} satisfies {wanted} but has no download for {}",
                    display(runtime),
                    lines.join(", "),
                    os_name(os)
                )
            };
            issues.push(ResolutionIssue {
                code: IssueCode::NoCompatibleVersion,
                message,
                unavailable_lines: lines,
            });
        }
    }

    RuntimeResolution {
        runtime: runtime.to_owned(),
        basis: spec.basis,
        constraint: spec.text,
        recommended,
        installed_matches,
        issues,
        conflicts_with,
    }
}

fn database_suggestion(
    report: &DetectionReport,
    installed: &[InstalledRuntime],
    available: &[RuntimeCatalog],
    os: Os,
) -> Option<DatabaseSuggestion> {
    let hint = report.database.as_ref()?;
    let newest_installed = |runtime: &str, line: &str| -> Option<String> {
        installed
            .iter()
            .filter(|r| r.runtime == runtime)
            .filter_map(|r| RuntimeVersion::parse(&r.version).map(|v| (r, v)))
            .filter(|(_, v)| v.is_on_line(line))
            .max_by(|a, b| a.1.cmp(&b.1))
            .map(|(r, _)| r.version.clone())
    };
    let suggest = |runtime: &str, line: String, note: DbNote| DatabaseSuggestion {
        engine: hint.engine,
        installed_version: newest_installed(runtime, &line),
        runtime: Some(runtime.to_owned()),
        line: Some(line),
        note,
    };
    let none = |note: DbNote| DatabaseSuggestion {
        engine: hint.engine,
        runtime: None,
        line: None,
        installed_version: None,
        note,
    };
    Some(match hint.engine {
        DbEngine::Mysql => suggest("mysql", DEFAULT_MYSQL_LINE.to_owned(), DbNote::None),
        DbEngine::Mariadb if os == Os::Macos => suggest(
            "mysql",
            DEFAULT_MYSQL_LINE.to_owned(),
            DbNote::MariadbNotOnMacos,
        ),
        DbEngine::Mariadb => {
            // The newest line that is still supported and can be installed here.
            let line = rows(available, "mariadb")
                .iter()
                .find(|row| row.available && row.channel == Channel::Stable && !row.eol)
                .map(|row| row.line.clone());
            match line {
                Some(line) => suggest("mariadb", line, DbNote::None),
                None => none(DbNote::None),
            }
        }
        DbEngine::Pgsql => none(DbNote::PostgresNotSupported),
        DbEngine::Sqlite => none(DbNote::SqliteNeedsNothing),
        DbEngine::Sqlsrv => none(DbNote::SqlServerNotSupported),
        DbEngine::Unknown => none(DbNote::NoDatabaseSpecified),
    })
}

/// Decide which PHP and Node to use for a project, what to do about its extensions and which database to suggest.
///
/// `available` is what the registry offers for this computer (`Registry::catalog`). A requirement that nothing satisfies
/// is not an error: the [`RuntimeResolution`] has no recommendation and says why in its `issues`.
pub fn resolve(
    report: &DetectionReport,
    installed: &[InstalledRuntime],
    available: &[RuntimeCatalog],
    data: &DetectionData,
    prefs: &ResolvePrefs,
) -> Resolution {
    let world = World {
        installed,
        available,
        data,
        os: prefs.os,
    };
    let php = resolve_runtime(
        &world,
        "php",
        report.php.as_ref(),
        prefs.php_override.as_deref(),
        prefs.default_php_line.as_deref(),
    );
    let node = (report.node.is_some() || prefs.node_override.is_some()).then(|| {
        resolve_runtime(
            &world,
            "node",
            report.node.as_ref(),
            prefs.node_override.as_deref(),
            prefs.default_node_line.as_deref(),
        )
    });
    let extensions = report
        .php_extensions
        .iter()
        .map(|ext| ExtensionStatus {
            name: ext.name.clone(),
            ini_name: ext.ini_name.clone(),
            needed_by: ext.needed_by.clone(),
            availability: if prefs.os == Os::Windows {
                ext.windows
            } else {
                ext.unix
            },
        })
        .collect();
    Resolution {
        php,
        node,
        extensions,
        database: database_suggestion(report, installed, available, prefs.os),
    }
}
