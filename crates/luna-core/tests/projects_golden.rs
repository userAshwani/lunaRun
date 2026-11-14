//! Detection and resolution against fixture projects (docs/PROJECT-DETECTION.md section 8).
//!
//! `tests/fixtures/projects/<name>/` is a minimal fake project (only the files that matter, no `vendor/`), and its
//! `expected.json` holds what `detect` must say and what `resolve` must decide in two fixed worlds, a Windows PC and a
//! Mac. The worlds are built **here**, not read from the live registry, so the answers do not move when upstream
//! releases a new PHP. `expected.json` is compared as data; to rewrite the files after a deliberate change run the test
//! with `LUNA_UPDATE_GOLDEN=1` and **read the diff**: a golden file is only worth what the person who accepted it checked.
//!
//! Besides the golden files this holds the properties that matter more than any single answer: detection never writes
//! into the project (FR-PJ-05), never lets a password out (NFR-06), and never panics, whatever the files contain.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod projects_world;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use luna_core::diagnostics::Os;
use luna_core::projects::{ResolvePrefs, detect, resolve};
use projects_world::{catalogs, data, installed};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("projects")
}

fn resolution(report: &luna_core::projects::DetectionReport, os: Os) -> Value {
    let prefs = ResolvePrefs::new(os);
    serde_json::to_value(resolve(
        report,
        &installed(os),
        &catalogs(os),
        &data(),
        &prefs,
    ))
    .unwrap()
}

/// What a fixture's `expected.json` holds.
fn actual(project: &Path) -> Value {
    let report = detect(project, &data());
    json!({
        "detect": serde_json::to_value(&report).unwrap(),
        "resolve": { "windows": resolution(&report, Os::Windows), "macos": resolution(&report, Os::Macos) },
    })
}

// ---------------------------------------------------------------------------------------------
// The golden files
// ---------------------------------------------------------------------------------------------

fn project_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// The first place two JSON values differ, as a path, so a failure says where to look.
fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                let here = format!("{path}/{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => {
                        if let Some(found) = first_difference(x, y, &here) {
                            return Some(found);
                        }
                    }
                    (Some(x), None) => {
                        return Some(format!("{here}: expected {x}, missing in the result"));
                    }
                    (None, Some(y)) => {
                        return Some(format!("{here}: not expected, the result has {y}"));
                    }
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                let here = format!("{path}/{i}");
                match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => {
                        if let Some(found) = first_difference(x, y, &here) {
                            return Some(found);
                        }
                    }
                    (Some(x), None) => {
                        return Some(format!(
                            "{here}: expected {x}, the result has no such entry"
                        ));
                    }
                    (None, Some(y)) => {
                        return Some(format!("{here}: not expected, the result has {y}"));
                    }
                    (None, None) => {}
                }
            }
            None
        }
        (a, b) if a == b => None,
        (a, b) => Some(format!("{path}: expected {a}, got {b}")),
    }
}

#[test]
fn every_fixture_matches_its_golden_file() {
    let update = std::env::var("LUNA_UPDATE_GOLDEN").is_ok();
    let mut failures = Vec::new();
    for dir in project_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let result = actual(&dir);
        let golden = fixtures().join(format!("{name}.expected.json"));
        if update {
            fs::write(
                &golden,
                serde_json::to_string_pretty(&result).unwrap() + "\n",
            )
            .unwrap();
            continue;
        }
        let Ok(text) = fs::read_to_string(&golden) else {
            failures.push(format!(
                "{name}: no {} (run with LUNA_UPDATE_GOLDEN=1, then read it)",
                golden.display()
            ));
            continue;
        };
        let expected: Value = serde_json::from_str(&text).unwrap();
        if let Some(difference) = first_difference(&expected, &result, "") {
            failures.push(format!("{name}: {difference}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixtures differ from their golden files:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_golden_file_has_a_fixture_and_every_fixture_a_golden_file() {
    let dirs: Vec<String> = project_dirs()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let goldens: Vec<String> = fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|n| n.strip_suffix(".expected.json").map(str::to_owned))
        .collect();
    for dir in &dirs {
        assert!(goldens.contains(dir), "fixture {dir} has no golden file");
    }
    for golden in &goldens {
        assert!(
            dirs.contains(golden),
            "golden file for {golden} has no fixture"
        );
    }
    assert!(
        dirs.len() >= 20,
        "the fixtures of the specification and the ones the real world needs: {}",
        dirs.len()
    );
}

// ---------------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------------

/// Every file under `dir` with its bytes and its size and modification time, in order.
fn snapshot(dir: &Path) -> BTreeMap<String, (u64, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, (u64, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(format!("{relative}/"), (0, Vec::new()));
                walk(base, &path, out);
            } else {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                let stamp = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64;
                out.insert(relative, (stamp, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn detection_reads_and_never_writes() {
    let temp = tempfile::TempDir::new().unwrap();
    for dir in project_dirs() {
        let copy = temp.path().join(dir.file_name().unwrap());
        copy_dir(&dir, &copy);
        let before = snapshot(&copy);
        let _ = detect(&copy, &data());
        let _ = detect(&copy, &data());
        assert_eq!(snapshot(&copy), before, "detect changed {}", dir.display());
    }
}

#[test]
fn a_password_or_any_other_secret_never_appears_in_a_report_or_a_resolution() {
    let secrets = [
        "secret123-never-in-a-report",
        "also-secret",
        "hunter2hunter2",
        "do-not-log-me",
    ];
    for dir in project_dirs() {
        let report = detect(&dir, &data());
        let text = format!(
            "{}{:?}{}",
            serde_json::to_string(&report).unwrap(),
            report,
            serde_json::to_string(&resolution(&report, Os::Windows)).unwrap()
        );
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{secret} leaked from {}",
                dir.display()
            );
        }
    }
    // The golden files are what the page would be given: they must not hold one either.
    for dir in project_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(fixtures().join(format!("{name}.expected.json"))).unwrap();
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{secret} is in the golden file of {name}"
            );
        }
    }
}

#[test]
fn a_folder_that_is_not_there_is_a_warning_not_a_failure() {
    let temp = tempfile::TempDir::new().unwrap();
    let missing = detect(&temp.path().join("nope"), &data());
    assert_eq!(missing.warnings.len(), 1);
    assert_eq!(
        serde_json::to_value(&missing).unwrap()["warnings"][0]["code"],
        "PROJECT_FOLDER_MISSING"
    );
    assert!(missing.php.is_none() && missing.node.is_none() && missing.framework.is_none());
    assert_eq!(missing.doc_root.path, ".");

    let file = temp.path().join("a-file");
    fs::write(&file, "x").unwrap();
    assert_eq!(
        serde_json::to_value(detect(&file, &data())).unwrap()["warnings"][0]["code"],
        "PROJECT_FOLDER_MISSING"
    );
    // And the answer is still resolvable: nothing required, so the defaults.
    let report = detect(&temp.path().join("nope"), &data());
    let resolved = resolve(
        &report,
        &installed(Os::Windows),
        &catalogs(Os::Windows),
        &data(),
        &ResolvePrefs::new(Os::Windows),
    );
    assert!(resolved.php.recommended.is_some());
}

/// A small deterministic generator: the same garbage on every run and every machine.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() >> 24) as u8).collect()
    }
}

#[test]
fn detection_survives_whatever_the_files_contain() {
    let names = [
        "composer.json",
        "composer.lock",
        "package.json",
        ".php-version",
        ".nvmrc",
        ".node-version",
        ".env",
        ".env.example",
    ];
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let deep = format!("{}1{}", "[".repeat(20_000), "]".repeat(20_000));
    let huge_number = format!("{{\"require\":{{\"php\":\"^{}.1\"}}}}", "9".repeat(5_000));
    let long_constraint = format!(
        "{{\"require\":{{\"php\":\"{}\"}}}}",
        "^8.1 || ".repeat(20_000) + "^8.2"
    );
    let payloads: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"\xEF\xBB\xBF".to_vec(),
        b"null".to_vec(),
        b"[]".to_vec(),
        b"\"just a string\"".to_vec(),
        b"{\"require\": 5, \"packages\": 7, \"engines\": [], \"config\": \"x\", \"volta\": null}".to_vec(),
        b"{\"require\": {\"php\": 8}, \"packages\": [1, null, \"x\", {\"name\": 3, \"require\": {\"php\": []}}]}".to_vec(),
        b"{\"packages\": [{\"name\": \"laravel/framework\", \"version\": \"dev-main\", \"require\": {\"php\": \"^\"}}], \"platform\": {\"php\": \"~\"}}".to_vec(),
        deep.into_bytes(),
        huge_number.into_bytes(),
        long_constraint.into_bytes(),
        noise.bytes(4096),
        noise.bytes(1),
        b"\n\n\n   \n".to_vec(),
        b"lts/\nlts/*\nnode\n".to_vec(),
        "\u{0}\u{0}\u{0}DB_HOST=\r\r".as_bytes().to_vec(),
        b"DB_PORT=99999999999999999999\nDB_CONNECTION=\nDB_HOST\n=\n".to_vec(),
        "d\u{e9}j\u{e0} vu \u{1F600}".as_bytes().to_vec(),
    ];
    let temp = tempfile::TempDir::new().unwrap();
    let mut count = 0;
    for payload in &payloads {
        for name in names {
            let dir = temp.path().join(format!("p{count}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(name), payload).unwrap();
            // Garbage in every file at once, too.
            if name == "composer.json" {
                for other in names {
                    fs::write(dir.join(other), payload).unwrap();
                }
                fs::write(dir.join("artisan"), "").unwrap();
            }
            let report = detect(&dir, &data());
            let resolved = resolve(
                &report,
                &installed(Os::Windows),
                &catalogs(Os::Windows),
                &data(),
                &ResolvePrefs::new(Os::Windows),
            );
            let _ = serde_json::to_string(&resolved).unwrap();
            assert_eq!(report.inputs_hash.len(), 64);
            count += 1;
        }
    }
    assert!(count > 100, "{count} garbage projects were read");
}

#[test]
fn the_inputs_hash_follows_the_inputs() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = temp.path().join("p");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("composer.json"),
        "{\"require\":{\"php\":\"^8.2\"}}",
    )
    .unwrap();
    let first = detect(&project, &data()).inputs_hash;
    assert_eq!(
        first,
        detect(&project, &data()).inputs_hash,
        "reading twice gives the same hash"
    );
    fs::write(
        project.join("composer.json"),
        "{\"require\":{\"php\":\"^8.3\"}}",
    )
    .unwrap();
    let second = detect(&project, &data()).inputs_hash;
    assert_ne!(first, second, "a changed input");
    fs::write(project.join(".env"), "DB_PASSWORD=changed\n").unwrap();
    assert_ne!(
        second,
        detect(&project, &data()).inputs_hash,
        "a new input, even one nothing is read from"
    );
    fs::write(project.join("README.md"), "not an input").unwrap();
    let with_env = detect(&project, &data()).inputs_hash;
    fs::write(project.join("README.md"), "still not an input").unwrap();
    assert_eq!(
        with_env,
        detect(&project, &data()).inputs_hash,
        "a file that is not an input changes nothing"
    );
}
