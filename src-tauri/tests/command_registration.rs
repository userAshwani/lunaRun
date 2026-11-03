//! Every IPC command has to be registered in four places or it fails at runtime with a permission error that
//! unit tests, which mock the backend, never see (CLAUDE.md "Adding an IPC command"):
//!
//! 1. the function in `src/commands.rs`,
//! 2. its name in `COMMANDS` in `build.rs`,
//! 3. its name in `generate_handler!` in `src/lib.rs`,
//! 4. `allow-<kebab-name>` in `capabilities/default.json`.
//!
//! This reads those four files and checks that they name exactly the same set.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
}

/// `jobs_list` becomes `jobs-list`, the form permissions use.
fn kebab(name: &str) -> String {
    name.replace('_', "-")
}

/// The text between `start` and the next `end` after it.
fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text
        .find(start)
        .unwrap_or_else(|| panic!("{start:?} not found"))
        + start.len();
    let to = text[from..].find(end).unwrap() + from;
    &text[from..to]
}

fn functions_in_commands() -> BTreeSet<String> {
    let source = read("src/commands.rs");
    let mut names = BTreeSet::new();
    let mut after_attribute = false;
    for line in source.lines().map(str::trim) {
        if line.starts_with("#[tauri::command") {
            after_attribute = true;
        } else if after_attribute && !line.starts_with('#') && !line.starts_with("///") {
            let signature = line
                .strip_prefix("pub async fn ")
                .or_else(|| line.strip_prefix("pub fn "))
                .unwrap_or_else(|| panic!("a #[tauri::command] must be a pub fn: {line:?}"));
            names.insert(signature.split('(').next().unwrap().to_owned());
            after_attribute = false;
        }
    }
    names
}

fn names_in_build_rs() -> BTreeSet<String> {
    between(&read("build.rs"), "const COMMANDS: &[&str] = &[", "];")
        .split(',')
        .map(|part| part.trim().trim_matches('"').to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

fn names_in_handler() -> BTreeSet<String> {
    between(&read("src/lib.rs"), "generate_handler![", "]")
        .split(',')
        .map(|part| part.trim().trim_start_matches("commands::").to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

fn permissions() -> BTreeSet<String> {
    let capability: serde_json::Value =
        serde_json::from_str(&read("capabilities/default.json")).unwrap();
    capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str())
        .filter_map(|p| p.strip_prefix("allow-"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_command_is_registered_everywhere_and_nowhere_else() {
    let functions = functions_in_commands();
    assert!(functions.len() >= 6, "found {functions:?}");

    assert_eq!(names_in_build_rs(), functions, "COMMANDS in build.rs");
    assert_eq!(names_in_handler(), functions, "generate_handler! in lib.rs");
    let granted: BTreeSet<String> = functions.iter().map(|f| kebab(f)).collect();
    assert_eq!(
        permissions(),
        granted,
        "allow-* in capabilities/default.json"
    );
}

#[test]
fn the_runtimes_commands_are_among_them() {
    let functions = functions_in_commands();
    for command in [
        "runtimes_available",
        "runtimes_installed",
        "registry_refresh",
    ] {
        assert!(functions.contains(command), "{command}");
    }
}
