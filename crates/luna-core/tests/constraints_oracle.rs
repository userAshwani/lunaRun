//! The constraint parsers against the real implementations (docs/PROJECT-DETECTION.md section 6).
//!
//! `tests/fixtures/constraints/composer.json` was made by asking Composer's own semver library, and `npm.json` by asking
//! node-semver, which of a list of versions satisfy each of a few hundred constraints, and whether the constraint is
//! accepted at all (`scripts/constraint-oracle/generate.mjs`). Every answer in those files is held here: a constraint the
//! real implementation refuses must be refused, and one it accepts must match **exactly** the same versions.
//!
//! The few places where lunaRun differs on purpose are listed below with the reason. Each is asserted to **still differ**,
//! so the list cannot go stale: when a difference is removed, this test says so.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use luna_core::constraints::{ConstraintWarning, Parsed, parse_composer, parse_npm};
use luna_core::registry::RuntimeVersion;
use serde_json::Value;

/// Composer accepts these and lunaRun does not, or reads them differently, on purpose (D-034).
const COMPOSER_DIFFERENCES: &[(&str, &str)] = &[
    (
        "dev-main",
        "a development branch cannot be compared with a PHP version: unconstrained with a warning, not 'matches nothing'",
    ),
    (
        "dev-master",
        "a development branch: unconstrained with a warning",
    ),
    (
        "dev-feature/x",
        "a development branch: unconstrained with a warning",
    ),
    (
        "dev-main@dev",
        "a development branch with a flag: unconstrained with a warning",
    ),
    (
        ">=dev-main",
        "a comparison with a development branch: unconstrained with a warning",
    ),
    (
        "dev-main as 8.1.0",
        "a development branch with an alias: unconstrained with a warning",
    ),
    (
        "dev-main dev-other",
        "development branches: unconstrained with a warning",
    ),
    (
        "8.1.x-dev",
        "a development branch: unconstrained with a warning",
    ),
    (
        "8.1.x-DEV",
        "a development branch: unconstrained with a warning",
    ),
    (
        "1.x-dev",
        "a development branch: unconstrained with a warning",
    ),
    (
        "8.x-dev",
        "a development branch: unconstrained with a warning",
    ),
    (
        "8.*-dev",
        "a development branch: unconstrained with a warning",
    ),
    (
        "1.2.*-dev",
        "a development branch: unconstrained with a warning",
    ),
    (
        "8.1.2.x-dev",
        "a development branch: unconstrained with a warning",
    ),
];

/// Composer accepts these and lunaRun refuses them: sloppy spellings.
const COMPOSER_STRICTER: &[(&str, &str)] = &[("8.1.", "a trailing dot")];

/// node-semver accepts these and lunaRun reads them differently on purpose.
const NPM_DIFFERENCES: &[(&str, &str)] = &[];

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("constraints")
        .join(name);
    serde_json::from_str(
        &fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

fn versions(file: &Value) -> Vec<RuntimeVersion> {
    file["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let text = v.as_str().unwrap();
            RuntimeVersion::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
        })
        .collect()
}

fn texts(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}

/// Run every case of `file` through `parse` and describe each disagreement.
fn disagreements(
    file: &Value,
    parse: impl Fn(&str) -> Result<Parsed, luna_core::constraints::ConstraintError>,
    differences: &[(&str, &str)],
    stricter: &[(&str, &str)],
) -> Vec<String> {
    let all = versions(file);
    let mut problems = Vec::new();
    let mut still_different: Vec<&str> = Vec::new();
    let mut still_stricter: Vec<&str> = Vec::new();
    for case in file["cases"].as_array().unwrap() {
        let constraint = case["constraint"].as_str().unwrap();
        let ours = parse(constraint);
        if case["valid"] == Value::Bool(false) {
            if let Ok(parsed) = &ours {
                problems.push(format!(
                    "{constraint:?}: refused by the real implementation, accepted here as {}",
                    parsed.req
                ));
            }
            continue;
        }
        let Ok(parsed) = ours else {
            if stricter.iter().any(|(c, _)| *c == constraint) {
                still_stricter.push(constraint);
            } else {
                problems.push(format!(
                    "{constraint:?}: accepted by the real implementation ({}), refused here: {}",
                    case["normalized"].as_str().unwrap_or("?"),
                    parse(constraint).unwrap_err()
                ));
            }
            continue;
        };
        let expected = texts(&case["matches"]);
        let got: Vec<&str> = all
            .iter()
            .filter(|v| parsed.req.matches(v))
            .map(RuntimeVersion::as_str)
            .collect();
        if got == expected {
            continue;
        }
        if differences.iter().any(|(c, _)| *c == constraint) {
            still_different.push(constraint);
        } else {
            let extra: Vec<&&str> = got.iter().filter(|v| !expected.contains(v)).collect();
            let missing: Vec<&&str> = expected.iter().filter(|v| !got.contains(v)).collect();
            problems.push(format!(
                "{constraint:?} (real: {}; here: {}): too many {extra:?}, missing {missing:?}",
                case["normalized"].as_str().unwrap_or("?"),
                parsed.req
            ));
        }
    }
    for (constraint, reason) in differences {
        if !still_different.contains(constraint) {
            problems.push(format!("{constraint:?} is listed as a difference ({reason}) but now agrees: remove it from the list"));
        }
    }
    for (constraint, reason) in stricter {
        if !still_stricter.contains(constraint) {
            problems.push(format!("{constraint:?} is listed as stricter ({reason}) but is now accepted: remove it from the list"));
        }
    }
    problems
}

#[test]
fn composer_constraints_match_composers_own_answers() {
    let file = fixture("composer.json");
    assert!(
        file["cases"].as_array().unwrap().len() > 300,
        "the table is large"
    );
    let problems = disagreements(
        &file,
        parse_composer,
        COMPOSER_DIFFERENCES,
        COMPOSER_STRICTER,
    );
    assert!(
        problems.is_empty(),
        "{} disagreements with Composer:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn npm_ranges_match_node_semvers_own_answers() {
    let file = fixture("npm.json");
    assert!(
        file["cases"].as_array().unwrap().len() > 300,
        "the table is large"
    );
    let problems = disagreements(&file, parse_npm, NPM_DIFFERENCES, &[]);
    assert!(
        problems.is_empty(),
        "{} disagreements with node-semver:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn the_differences_from_composer_come_with_a_warning_a_person_can_show() {
    for (constraint, _) in COMPOSER_DIFFERENCES {
        let parsed = parse_composer(constraint).unwrap();
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| matches!(w, ConstraintWarning::DevBranch(_))),
            "{constraint:?} must warn about the development branch"
        );
    }
}

#[test]
fn the_tables_exercise_both_accepting_and_refusing_and_every_kind_of_answer() {
    for name in ["composer.json", "npm.json"] {
        let file = fixture(name);
        let cases = file["cases"].as_array().unwrap();
        let accepted = cases
            .iter()
            .filter(|c| c["valid"] == Value::Bool(true))
            .count();
        let refused = cases.len() - accepted;
        assert!(
            accepted > 200 && refused > 15,
            "{name}: {accepted} accepted, {refused} refused"
        );
        let empty = cases
            .iter()
            .filter(|c| {
                c["valid"] == Value::Bool(true) && c["matches"].as_array().unwrap().is_empty()
            })
            .count();
        let releases = file["versions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| !v.as_str().unwrap().contains('-'))
            .count();
        let everything = cases
            .iter()
            .filter(|c| {
                c["valid"] == Value::Bool(true)
                    && c["matches"].as_array().unwrap().len() == releases
            })
            .count();
        assert!(
            empty > 0 && everything > 0,
            "{name}: constraints that hold nothing ({empty}) and everything ({everything}) are in the table"
        );
    }
}
