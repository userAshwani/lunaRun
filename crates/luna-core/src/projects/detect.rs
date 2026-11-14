//! `detect`: read a project folder and say what it asks for (docs/PROJECT-DETECTION.md sections 2 to 5).
//!
//! Read-only and never failing: every problem with a file is a [`Warning`], and detection goes on with what it has.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::data::DetectionData;
use super::dotenv::read_keys;
use super::files::{Inputs, warning};
use super::{
    Availability, DbEngine, DbHint, DetectionReport, DocRoot, ExtRequirement, Framework,
    FrameworkKind, Pin, Preferred, Requirement, Selector, ServiceHint, Source, SourceKind, Warning,
    WarningCode,
};
use crate::constraints::{Bound, VersionReq, parse_composer, parse_npm};
use crate::registry::RuntimeVersion;

/// The keys read from `.env`, and no others. `DB_PASSWORD` is not among them, on purpose.
const ENV_KEYS: [&str; 11] = [
    "DB_CONNECTION",
    "DB_HOST",
    "DB_PORT",
    "DB_DATABASE",
    "DB_USERNAME",
    "REDIS_HOST",
    "CACHE_STORE",
    "SESSION_DRIVER",
    "QUEUE_CONNECTION",
    "MAIL_HOST",
    "MAIL_PORT",
];

/// Read `project_dir` and report what it needs. Never writes, never runs project code, never fails.
pub fn detect(project_dir: &Path, data: &DetectionData) -> DetectionReport {
    if !project_dir.is_dir() {
        return DetectionReport {
            framework: None,
            doc_root: DocRoot {
                path: ".".to_owned(),
                evidence: "the project folder does not exist".to_owned(),
            },
            php: None,
            php_extensions: Vec::new(),
            node: None,
            package_manager: None,
            database: None,
            services_hints: Vec::new(),
            warnings: vec![warning(
                WarningCode::ProjectFolderMissing,
                None,
                format!("{} is not a folder", project_dir.display()),
                Vec::new(),
            )],
            inputs_hash: Inputs::new(project_dir).finish().0,
        };
    }

    let mut inputs = Inputs::new(project_dir);
    let composer = inputs.json("composer.json");
    let lock = inputs.json("composer.lock");
    let package = inputs.json("package.json");
    let php_version = inputs.text(".php-version");
    let nvmrc = inputs.text(".nvmrc");
    let node_version = inputs.text(".node-version");
    // Both env files go into the hash; only the whitelisted keys of one of them are read, and the text is dropped here.
    let env = {
        let env_text = inputs.text(".env");
        let example_text = inputs.text(".env.example");
        match (env_text, example_text) {
            (Some(text), _) => Some((".env", read_keys(&text, &ENV_KEYS))),
            (None, Some(text)) => Some((".env.example", read_keys(&text, &ENV_KEYS))),
            (None, None) => None,
        }
    };
    let has_artisan = inputs.exists("artisan", false);
    let has_public_index = inputs.exists("public/index.php", false);
    let has_root_index = inputs.exists("index.php", false);
    let has_public_dir = inputs.exists("public", true);

    let mut warnings: Vec<Warning> = Vec::new();
    let framework = detect_framework(has_artisan, composer.as_ref(), lock.as_ref(), &mut warnings);
    let php = detect_php(
        composer.as_ref(),
        lock.as_ref(),
        php_version.as_deref(),
        framework.as_ref(),
        data,
        &mut warnings,
    );
    let php_extensions =
        detect_extensions(composer.as_ref(), lock.as_ref(), framework.is_some(), data);
    let node = detect_node(
        package.as_ref(),
        nvmrc.as_deref(),
        node_version.as_deref(),
        data,
        &mut warnings,
    );
    let package_manager = package
        .as_ref()
        .and_then(|p| p.get("packageManager"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (database, services_hints) = match &env {
        Some((file, keys)) => (database_hint(file, keys), service_hints(keys)),
        None => (None, Vec::new()),
    };
    let doc_root = doc_root(
        framework.is_some(),
        has_public_index,
        has_root_index,
        has_public_dir,
    );

    let (inputs_hash, mut all_warnings) = inputs.finish();
    all_warnings.extend(warnings);
    DetectionReport {
        framework,
        doc_root,
        php,
        php_extensions,
        node,
        package_manager,
        database,
        services_hints,
        warnings: all_warnings,
        inputs_hash,
    }
}

// ---------------------------------------------------------------------------------------------
// Framework and document root
// ---------------------------------------------------------------------------------------------

/// The `packages` and `packages-dev` entries of a lock file, with the section they are in.
fn lock_packages(lock: &Value) -> Vec<(&'static str, &str, &Value)> {
    let mut all = Vec::new();
    for section in ["packages", "packages-dev"] {
        if let Some(list) = lock.get(section).and_then(Value::as_array) {
            for package in list {
                let name = package.get("name").and_then(Value::as_str).unwrap_or("?");
                all.push((section, name, package));
            }
        }
    }
    all
}

fn clean_version(text: &str) -> &str {
    text.trim().trim_start_matches(['v', 'V'])
}

fn locked_version(lock: &Value, name: &str) -> Option<RuntimeVersion> {
    lock_packages(lock)
        .into_iter()
        .find(|(_, package, _)| *package == name)
        .and_then(|(_, _, p)| p.get("version"))
        .and_then(Value::as_str)
        .and_then(|v| RuntimeVersion::parse(clean_version(v)))
}

fn detect_framework(
    has_artisan: bool,
    composer: Option<&Value>,
    lock: Option<&Value>,
    warnings: &mut Vec<Warning>,
) -> Option<Framework> {
    if !has_artisan {
        return None;
    }
    let in_lock = lock.is_some_and(|l| {
        lock_packages(l)
            .iter()
            .any(|(_, name, _)| *name == "laravel/framework")
    });
    let in_composer = composer.is_some_and(|c| {
        ["require", "require-dev"].iter().any(|section| {
            c.get(*section)
                .and_then(|r| r.get("laravel/framework"))
                .is_some()
        })
    });
    if !in_lock && !in_composer {
        return None;
    }
    let mut evidence = vec!["artisan".to_owned()];
    if in_lock {
        evidence.push("laravel/framework in composer.lock".to_owned());
    } else {
        evidence.push("laravel/framework in composer.json".to_owned());
    }
    let version = lock
        .and_then(|l| locked_version(l, "laravel/framework"))
        .map(|v| v.as_str().to_owned());
    if version.is_none() {
        warnings.push(warning(
            WarningCode::FrameworkVersionUnknown,
            Some("composer.lock"),
            "Laravel is there but composer.lock does not say which version",
            Vec::new(),
        ));
    }
    Some(Framework {
        kind: FrameworkKind::Laravel,
        version,
        evidence,
    })
}

fn doc_root(laravel: bool, public_index: bool, root_index: bool, public_dir: bool) -> DocRoot {
    let (path, evidence) = if laravel {
        ("public", "Laravel serves from public/")
    } else if public_index {
        ("public", "public/index.php exists")
    } else if root_index {
        (".", "index.php is in the project folder")
    } else if public_dir {
        ("public", "there is a public/ folder")
    } else {
        (".", "nothing says otherwise, so the project folder itself")
    };
    DocRoot {
        path: path.to_owned(),
        evidence: evidence.to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// PHP
// ---------------------------------------------------------------------------------------------

/// One constraint of a project file with its meaning.
struct Part {
    source: Source,
    req: VersionReq,
}

fn composer_part(
    warnings: &mut Vec<Warning>,
    kind: SourceKind,
    file: &str,
    path: &str,
    raw: &str,
) -> Option<Part> {
    match parse_composer(raw) {
        Ok(parsed) => {
            if !parsed.warnings.is_empty() {
                warnings.push(warning(
                    WarningCode::DevBranch,
                    Some(file),
                    format!(
                        "{path} names a development branch, which does not restrict the version"
                    ),
                    vec![path.to_owned(), raw.to_owned()],
                ));
            }
            Some(Part {
                source: Source {
                    kind,
                    file: file.to_owned(),
                    path: path.to_owned(),
                    raw: raw.to_owned(),
                    constraint: parsed.req.to_string(),
                },
                req: parsed.req,
            })
        }
        Err(error) => {
            warnings.push(warning(
                WarningCode::InvalidConstraint,
                Some(file),
                format!(
                    "{path} is not a version constraint Composer reads ({error}): it is ignored"
                ),
                vec![path.to_owned(), raw.to_owned()],
            ));
            None
        }
    }
}

/// `8.3` and `8.3.*` are the line, `8.3.14` is that version, `v8.3.14` the same: how `.php-version` and `.nvmrc`
/// write a version. The bool says it is a whole version.
pub(super) fn version_pin(text: &str) -> Option<(VersionReq, bool)> {
    let trimmed = clean_version(text);
    let mut numbers: Vec<&str> = Vec::new();
    let mut wildcard = false;
    for part in trimmed.split('.') {
        if matches!(part, "x" | "X" | "*") {
            wildcard = true;
        } else if wildcard || part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        } else {
            numbers.push(part);
        }
    }
    if numbers.is_empty() || numbers.len() > 3 || (numbers.len() == 3 && wildcard) {
        return None;
    }
    let exact = numbers.len() == 3;
    let spec = if exact {
        numbers.join(".")
    } else {
        format!("{}.*", numbers.join("."))
    };
    parse_composer(&spec).ok().map(|parsed| (parsed.req, exact))
}

/// The first line of a version file that is not empty.
fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

/// The sources that name the units that cannot all be satisfied.
fn conflicting(parts: &[Part], root: Option<usize>) -> Vec<String> {
    let describe = |part: &Part| {
        format!(
            "{} {} = {}",
            part.source.file, part.source.path, part.source.raw
        )
    };
    let mut found: Vec<usize> = Vec::new();
    if let Some(root) = root {
        found = (0..parts.len())
            .filter(|i| *i != root && parts[root].req.intersect(&parts[*i].req).is_empty())
            .collect();
    }
    if found.is_empty() {
        for i in 0..parts.len() {
            for j in (i + 1)..parts.len() {
                if parts[i].req.intersect(&parts[j].req).is_empty() {
                    for k in [i, j] {
                        if !found.contains(&k) {
                            found.push(k);
                        }
                    }
                }
            }
        }
    }
    if found.is_empty() {
        found = (0..parts.len()).collect();
    }
    found.iter().map(|i| describe(&parts[*i])).collect()
}

/// A Laravel version to look up in the table when the project files give no PHP constraint: the locked one, else the
/// lowest the `composer.json` constraint allows.
fn framework_version_for_table(
    framework: Option<&Framework>,
    composer: Option<&Value>,
) -> Option<(RuntimeVersion, String, String)> {
    let framework = framework?;
    if let Some(version) = &framework.version {
        let parsed = RuntimeVersion::parse(version)?;
        return Some((
            parsed,
            "composer.lock".to_owned(),
            format!("laravel/framework {version}"),
        ));
    }
    let raw = composer?
        .get("require")?
        .get("laravel/framework")?
        .as_str()?;
    let req = parse_composer(raw).ok()?.req;
    let low = match &req.intervals().first()?.lower {
        Bound::Included(v) | Bound::Excluded(v) => v.clone(),
        Bound::Unbounded => return None,
    };
    Some((
        low,
        "composer.json".to_owned(),
        format!("laravel/framework {raw}"),
    ))
}

fn detect_php(
    composer: Option<&Value>,
    lock: Option<&Value>,
    php_version_file: Option<&str>,
    framework: Option<&Framework>,
    data: &DetectionData,
    warnings: &mut Vec<Warning>,
) -> Option<Requirement> {
    let mut parts: Vec<Part> = Vec::new();
    let mut root: Option<usize> = None;

    // 5. composer.json require.php
    if let Some(raw) = composer
        .and_then(|c| c.get("require"))
        .and_then(|r| r.get("php"))
        .and_then(Value::as_str)
        && let Some(part) = composer_part(
            warnings,
            SourceKind::ComposerRequire,
            "composer.json",
            "require.php",
            raw,
        )
    {
        root = Some(parts.len());
        parts.push(part);
    }
    // 6. every package of the lock file
    if let Some(lock) = lock {
        for (section, name, package) in lock_packages(lock) {
            if let Some(raw) = package
                .get("require")
                .and_then(|r| r.get("php"))
                .and_then(Value::as_str)
            {
                let path = format!("{section}[{name}].require.php");
                if let Some(part) = composer_part(
                    warnings,
                    SourceKind::ComposerLock,
                    "composer.lock",
                    &path,
                    raw,
                ) {
                    parts.push(part);
                }
            }
        }
        // 7. the platform of the lock file, only when there is no composer.json to ask
        if composer.is_none()
            && let Some(raw) = lock
                .get("platform")
                .and_then(|p| p.get("php"))
                .and_then(Value::as_str)
            && let Some(part) = composer_part(
                warnings,
                SourceKind::ComposerLockPlatform,
                "composer.lock",
                "platform.php",
                raw,
            )
        {
            root = Some(parts.len());
            parts.push(part);
        }
    }

    // 4. config.platform.php: the version the team targets
    let preferred = composer
        .and_then(|c| c.get("config"))
        .and_then(|c| c.get("platform"))
        .and_then(|p| p.get("php"))
        .and_then(Value::as_str)
        .and_then(|raw| {
            let numbers: Vec<&str> = clean_version(raw).split('.').collect();
            let valid = numbers.len() >= 2
                && numbers
                    .iter()
                    .take(2)
                    .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if valid {
                Some(Preferred {
                    text: raw.to_owned(),
                    line: format!("{}.{}", numbers[0], numbers[1]),
                    file: "composer.json".to_owned(),
                    path: "config.platform.php".to_owned(),
                })
            } else {
                warnings.push(warning(
                    WarningCode::InvalidConstraint,
                    Some("composer.json"),
                    format!(
                        "config.platform.php is '{raw}', which is not a PHP version: it is ignored"
                    ),
                    vec!["config.platform.php".to_owned(), raw.to_owned()],
                ));
                None
            }
        });

    // The effective constraint of the Composer files: everything intersected, or the root alone when that is empty.
    let mut computed: Option<VersionReq> = None;
    if !parts.is_empty() {
        let everything = parts
            .iter()
            .fold(VersionReq::any(), |all, part| all.intersect(&part.req));
        if everything.is_empty() {
            let who = conflicting(&parts, root);
            warnings.push(warning(
                WarningCode::UnsatisfiablePhpConstraint,
                Some("composer.lock"),
                "no PHP version satisfies every requirement of the Composer files: the requirement of composer.json alone is used",
                who,
            ));
            let fallback = root.unwrap_or(0);
            computed = Some(parts[fallback].req.clone());
        } else {
            computed = Some(everything);
        }
    } else if let Some((version, file, raw)) = framework_version_for_table(framework, composer)
        && let Some(row) = data
            .laravel
            .iter()
            .find(|row| row.framework.matches(&version))
    {
        // 8. the framework table, only when nothing else says
        parts.push(Part {
            source: Source {
                kind: SourceKind::FrameworkTable,
                file: "laravel-php.json".to_owned(),
                path: format!("laravel[{}] (from {file}: {raw})", row.framework_text),
                raw: row.php_text.clone(),
                constraint: row.php.to_string(),
            },
            req: row.php.clone(),
        });
        computed = Some(row.php.clone());
    }

    let mut sources: Vec<Source> = Vec::new();
    // 3. .php-version: a pin that replaces the computed constraint
    let mut pin: Option<(Pin, VersionReq)> = None;
    if let Some(text) = php_version_file.and_then(first_line) {
        match version_pin(text) {
            Some((req, exact)) => {
                sources.push(Source {
                    kind: SourceKind::PhpVersionFile,
                    file: ".php-version".to_owned(),
                    path: String::new(),
                    raw: text.to_owned(),
                    constraint: req.to_string(),
                });
                pin = Some((
                    Pin {
                        text: text.to_owned(),
                        exact,
                        file: ".php-version".to_owned(),
                    },
                    req,
                ));
            }
            None => warnings.push(warning(
                WarningCode::InvalidPhpVersionFile,
                Some(".php-version"),
                format!(".php-version says '{text}', which is not a PHP version such as 8.3 or 8.3.14: it is ignored"),
                vec![text.to_owned()],
            )),
        }
    }
    if let Some(preferred) = &preferred {
        sources.push(Source {
            kind: SourceKind::ComposerPlatformConfig,
            file: preferred.file.clone(),
            path: preferred.path.clone(),
            raw: preferred.text.clone(),
            constraint: format!("line {}", preferred.line),
        });
    }
    sources.extend(parts.into_iter().map(|part| part.source));

    match (pin, computed) {
        (Some((pin, pin_req)), computed) => {
            if let Some(computed) = &computed
                && pin_req.intersect(computed).is_empty()
            {
                warnings.push(warning(
                    WarningCode::PinConflictsWithComposer,
                    Some(&pin.file),
                    format!(
                        "{} pins PHP {}, which the Composer files do not allow ({computed})",
                        pin.file, pin.text
                    ),
                    vec![pin.text.clone(), computed.to_string()],
                ));
            }
            Some(Requirement {
                constraint: pin_req.to_string(),
                req: pin_req,
                selector: Selector::Constraint,
                pin: Some(pin),
                computed: computed.map(|c| c.to_string()),
                preferred,
                sources,
            })
        }
        (None, Some(computed)) => Some(Requirement {
            constraint: computed.to_string(),
            req: computed,
            selector: Selector::Constraint,
            pin: None,
            computed: None,
            preferred,
            sources,
        }),
        (None, None) => preferred.map(|preferred| Requirement {
            constraint: VersionReq::any().to_string(),
            req: VersionReq::any(),
            selector: Selector::Constraint,
            pin: None,
            computed: None,
            preferred: Some(preferred),
            sources,
        }),
    }
}

// ---------------------------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------------------------

fn extension_names(requires: &Value) -> Vec<String> {
    requires
        .as_object()
        .map(|map| {
            map.keys()
                .filter_map(|key| key.strip_prefix("ext-"))
                .map(str::to_ascii_lowercase)
                .filter(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn detect_extensions(
    composer: Option<&Value>,
    lock: Option<&Value>,
    laravel: bool,
    data: &DetectionData,
) -> Vec<ExtRequirement> {
    let mut needed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut add = |name: String, by: &str| {
        needed.entry(name).or_default().insert(by.to_owned());
    };
    if let Some(composer) = composer {
        for section in ["require", "require-dev"] {
            if let Some(requires) = composer.get(section) {
                for name in extension_names(requires) {
                    add(name, "composer.json");
                }
            }
        }
    }
    if let Some(lock) = lock {
        for (_, package, value) in lock_packages(lock) {
            if let Some(requires) = value.get("require") {
                for name in extension_names(requires) {
                    add(name, package);
                }
            }
        }
    }
    if laravel {
        for name in &data.laravel_baseline {
            add(name.clone(), "Laravel");
        }
    }
    needed
        .into_iter()
        .map(|(name, by)| {
            let info = data.extensions.get(&name);
            ExtRequirement {
                ini_name: info
                    .and_then(|i| i.ini.clone())
                    .unwrap_or_else(|| name.clone()),
                zend_extension: info.is_some_and(|i| i.zend),
                windows: info.map_or(Availability::Unknown, |i| i.windows),
                unix: info.map_or(Availability::Unknown, |i| i.unix),
                needed_by: by.into_iter().collect(),
                name,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------------------------

/// What an `.nvmrc`, `.node-version` or `volta.node` says.
enum NodePin {
    Constraint { req: VersionReq, exact: bool },
    LatestLts,
    Latest,
}

enum PinProblem {
    Unsupported,
    UnknownCodename,
}

fn parse_node_pin(text: &str, data: &DetectionData) -> Result<NodePin, PinProblem> {
    let lower = text.trim().to_ascii_lowercase();
    if lower == "lts/*" {
        return Ok(NodePin::LatestLts);
    }
    if let Some(codename) = lower.strip_prefix("lts/") {
        let major = data
            .lts_major_of(codename)
            .ok_or(PinProblem::UnknownCodename)?;
        let (req, _) = version_pin(&major.to_string()).ok_or(PinProblem::Unsupported)?;
        return Ok(NodePin::Constraint { req, exact: false });
    }
    if matches!(lower.as_str(), "node" | "stable" | "latest" | "current") {
        return Ok(NodePin::Latest);
    }
    if let Some((req, exact)) = version_pin(&lower) {
        return Ok(NodePin::Constraint { req, exact });
    }
    parse_npm(&lower)
        .map(|parsed| NodePin::Constraint {
            req: parsed.req,
            exact: false,
        })
        .map_err(|_| PinProblem::Unsupported)
}

fn detect_node(
    package: Option<&Value>,
    nvmrc: Option<&str>,
    node_version: Option<&str>,
    data: &DetectionData,
    warnings: &mut Vec<Warning>,
) -> Option<Requirement> {
    let mut sources: Vec<Source> = Vec::new();

    // 6. engines.node, the constraint the files compute
    let mut computed: Option<VersionReq> = None;
    if let Some(raw) = package
        .and_then(|p| p.get("engines"))
        .and_then(|e| e.get("node"))
        .and_then(Value::as_str)
    {
        match parse_npm(raw) {
            Ok(parsed) => {
                sources.push(Source {
                    kind: SourceKind::Engines,
                    file: "package.json".to_owned(),
                    path: "engines.node".to_owned(),
                    raw: raw.to_owned(),
                    constraint: parsed.req.to_string(),
                });
                computed = Some(parsed.req);
            }
            Err(error) => warnings.push(warning(
                WarningCode::InvalidConstraint,
                Some("package.json"),
                format!("engines.node is not a range npm reads ({error}): it is ignored"),
                vec!["engines.node".to_owned(), raw.to_owned()],
            )),
        }
    }

    // 3, 4, 5: the first pin that says something
    let volta = package
        .and_then(|p| p.get("volta"))
        .and_then(|v| v.get("node"))
        .and_then(Value::as_str);
    let candidates = [
        (
            SourceKind::NvmRc,
            ".nvmrc",
            String::new(),
            nvmrc.and_then(first_line),
        ),
        (
            SourceKind::NodeVersionFile,
            ".node-version",
            String::new(),
            node_version.and_then(first_line),
        ),
        (
            SourceKind::Volta,
            "package.json",
            "volta.node".to_owned(),
            volta,
        ),
    ];
    let mut chosen: Option<(Pin, NodePin)> = None;
    for (kind, file, path, text) in candidates {
        let Some(text) = text else { continue };
        match parse_node_pin(text, data) {
            Ok(parsed) => {
                let constraint = match &parsed {
                    NodePin::Constraint { req, .. } => req.to_string(),
                    NodePin::LatestLts => "lts/*".to_owned(),
                    NodePin::Latest => "latest".to_owned(),
                };
                sources.insert(
                    0,
                    Source {
                        kind,
                        file: file.to_owned(),
                        path,
                        raw: text.to_owned(),
                        constraint,
                    },
                );
                if chosen.is_none() {
                    let exact = matches!(parsed, NodePin::Constraint { exact: true, .. });
                    chosen = Some((
                        Pin {
                            text: text.to_owned(),
                            exact,
                            file: file.to_owned(),
                        },
                        parsed,
                    ));
                }
            }
            Err(problem) => {
                let (code, why) = match problem {
                    PinProblem::UnknownCodename => (
                        WarningCode::UnknownLtsCodename,
                        "is not an LTS codename lunaRun knows",
                    ),
                    PinProblem::Unsupported => (
                        WarningCode::UnsupportedNodeVersion,
                        "is not a Node version lunaRun can read",
                    ),
                };
                warnings.push(warning(
                    code,
                    Some(file),
                    format!("{file} says '{text}', which {why}: it is ignored"),
                    vec![text.to_owned()],
                ));
            }
        }
    }

    match (chosen, computed) {
        (Some((pin, parsed)), computed) => {
            let (req, selector, constraint) = match parsed {
                NodePin::Constraint { req, .. } => {
                    if let Some(engines) = &computed
                        && req.intersect(engines).is_empty()
                    {
                        warnings.push(warning(
                            WarningCode::PinConflictsWithEngines,
                            Some(&pin.file),
                            format!(
                                "{} pins Node {}, which engines.node does not allow ({engines})",
                                pin.file, pin.text
                            ),
                            vec![pin.text.clone(), engines.to_string()],
                        ));
                    }
                    let text = req.to_string();
                    (req, Selector::Constraint, text)
                }
                NodePin::LatestLts => (VersionReq::any(), Selector::LatestLts, "lts/*".to_owned()),
                NodePin::Latest => (VersionReq::any(), Selector::Latest, "latest".to_owned()),
            };
            Some(Requirement {
                constraint,
                req,
                selector,
                pin: Some(pin),
                computed: computed.map(|c| c.to_string()),
                preferred: None,
                sources,
            })
        }
        (None, Some(computed)) => Some(Requirement {
            constraint: computed.to_string(),
            req: computed,
            selector: Selector::Constraint,
            pin: None,
            computed: None,
            preferred: None,
            sources,
        }),
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Database and the other services
// ---------------------------------------------------------------------------------------------

fn database_hint(file: &str, keys: &BTreeMap<String, String>) -> Option<DbHint> {
    let get = |key: &str| {
        keys.get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let connection = get("DB_CONNECTION");
    let (host, port, database, username) = (
        get("DB_HOST"),
        get("DB_PORT"),
        get("DB_DATABASE"),
        get("DB_USERNAME"),
    );
    if connection.is_none() && host.is_none() && port.is_none() && database.is_none() {
        return None;
    }
    let engine = match connection
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mysql") => DbEngine::Mysql,
        Some("mariadb") => DbEngine::Mariadb,
        Some("pgsql" | "postgres" | "postgresql") => DbEngine::Pgsql,
        Some("sqlite") => DbEngine::Sqlite,
        Some("sqlsrv") => DbEngine::Sqlsrv,
        _ => DbEngine::Unknown,
    };
    Some(DbHint {
        engine,
        connection,
        host,
        port: port.and_then(|p| p.parse::<u16>().ok()),
        database,
        username,
        file: file.to_owned(),
    })
}

fn service_hints(keys: &BTreeMap<String, String>) -> Vec<ServiceHint> {
    let get = |key: &str| keys.get(key).map(|v| v.trim()).filter(|v| !v.is_empty());
    let mut hints = Vec::new();
    let redis = [
        ("CACHE_STORE", "redis"),
        ("SESSION_DRIVER", "redis"),
        ("QUEUE_CONNECTION", "redis"),
    ]
    .iter()
    .find(|(key, wanted)| get(key).is_some_and(|v| v.eq_ignore_ascii_case(wanted)))
    .map(|(key, wanted)| format!("{key}={wanted}"))
    .or_else(|| get("REDIS_HOST").map(|host| format!("REDIS_HOST={host}")));
    if let Some(reason) = redis {
        hints.push(ServiceHint {
            service: "redis".to_owned(),
            reason,
        });
    }
    if let Some(host) = get("MAIL_HOST") {
        let local = ["127.0.0.1", "localhost", "mailpit", "mailhog"]
            .contains(&host.to_ascii_lowercase().as_str());
        if local {
            let reason = match get("MAIL_PORT") {
                Some(port) => format!("MAIL_HOST={host}, MAIL_PORT={port}"),
                None => format!("MAIL_HOST={host}"),
            };
            hints.push(ServiceHint {
                service: "mail".to_owned(),
                reason,
            });
        }
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::ENV_KEYS;

    /// Golden rule 7: the list of keys read from `.env` is the only thing between a password and the report, and the
    /// report never shows what was not read. A report test cannot see a key that is read and then dropped, so the list is
    /// held to the rule directly.
    #[test]
    fn the_env_whitelist_names_no_secret() {
        for key in ENV_KEYS {
            let upper = key.to_ascii_uppercase();
            for secret in ["PASSWORD", "SECRET", "TOKEN", "KEY", "CREDENTIAL", "PASS"] {
                assert!(
                    !upper.contains(secret),
                    "{key} looks like a secret ({secret})"
                );
            }
            assert!(
                key.starts_with("DB_")
                    || ["REDIS_", "CACHE_", "SESSION_", "QUEUE_", "MAIL_"]
                        .iter()
                        .any(|p| key.starts_with(p)),
                "{key} is not a connection or driver setting"
            );
        }
        assert!(ENV_KEYS.contains(&"DB_HOST") && ENV_KEYS.contains(&"DB_PORT"));
    }
}
