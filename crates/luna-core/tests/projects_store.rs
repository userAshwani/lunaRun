//! The project store (docs/ARCHITECTURE.md section 5; D-036): what it keeps, what it refuses, that it never writes into a
//! project folder or deletes anything, and that what the page sends cannot make it do the wrong thing.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod projects_world;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use luna_core::diagnostics::Os;
use luna_core::projects::{
    Basis, Binding, BindingChoice, BindingMode, DocRootChoice, ProjectDetection, ProjectPatch,
    ProjectStore, Surroundings,
};
use luna_core::{ErrorCode, LunaHome};
use projects_world::{catalogs, data, installed};
use tempfile::TempDir;

struct Bench {
    tmp: TempDir,
    home: LunaHome,
    store: ProjectStore,
}

/// A data folder with a space and a non-ASCII letter, and an empty store in it.
fn bench() -> Bench {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    home.ensure_layout().unwrap();
    let store = ProjectStore::open(&home).unwrap();
    Bench { tmp, home, store }
}

impl Bench {
    /// A project folder (with a space and a non-ASCII letter in its name) holding `files`.
    fn project(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.tmp.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn add(&self, dir: &Path) -> ProjectDetection {
        self.try_add(dir).unwrap()
    }

    fn try_add(&self, dir: &Path) -> luna_core::Result<ProjectDetection> {
        let (installed, available) = (installed(Os::Windows), catalogs(Os::Windows));
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: Os::Windows,
        };
        self.store.add(dir, &data(), &around)
    }

    fn inspect(&self, id: &str) -> ProjectDetection {
        let (installed, available) = (installed(Os::Windows), catalogs(Os::Windows));
        let around = Surroundings {
            installed: &installed,
            available: &available,
            os: Os::Windows,
        };
        self.store.inspect(id, &data(), &around).unwrap()
    }

    fn update(
        &self,
        id: &str,
        patch: &ProjectPatch,
    ) -> luna_core::Result<luna_core::projects::Project> {
        self.store.update(id, patch, &data())
    }

    fn file(&self) -> PathBuf {
        self.home.config_dir().join("projects.json")
    }
}

const LARAVEL: &[(&str, &str)] = &[
    ("artisan", ""),
    (
        "composer.json",
        r#"{"require":{"php":"^8.2","laravel/framework":"^11.0"}}"#,
    ),
    ("public/index.php", "<?php"),
];

fn use_php(mode: BindingMode, value: &str) -> ProjectPatch {
    ProjectPatch {
        php: Some(BindingChoice::Use {
            binding: Binding {
                mode,
                value: value.to_owned(),
            },
        }),
        ..ProjectPatch::default()
    }
}

fn use_node(mode: BindingMode, value: &str) -> ProjectPatch {
    ProjectPatch {
        node: Some(BindingChoice::Use {
            binding: Binding {
                mode,
                value: value.to_owned(),
            },
        }),
        ..ProjectPatch::default()
    }
}

/// Every file under `dir` with its bytes: what "the folder is untouched" is compared by.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(format!("{name}/"), Vec::new());
                walk(&path, base, out);
            } else {
                out.insert(name, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn code(result: luna_core::Result<impl std::fmt::Debug>) -> ErrorCode {
    result.unwrap_err().code
}

// ------------------------------------------------------------------------------------------------ add

#[test]
fn adding_a_folder_keeps_what_the_user_knows_and_reads_what_the_folder_says() {
    let b = bench();
    let dir = b.project("mon projet é", LARAVEL);
    let added = b.add(&dir);
    let p = &added.project;
    assert_eq!(p.name, "mon projet é");
    assert_eq!(p.path, dir.to_str().unwrap());
    assert_eq!((p.doc_root.as_str(), p.doc_root_custom), ("public", false));
    assert_eq!(
        p.framework.as_ref().unwrap().kind,
        luna_core::projects::FrameworkKind::Laravel
    );
    assert_eq!(p.id.len(), 32);
    assert!(p.id.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(
        p.last_detection.as_ref().unwrap().hash,
        added.report.inputs_hash
    );
    assert!(
        p.bindings.php.is_none() && p.bindings.node.is_none(),
        "nothing is bound by hand"
    );
    // And the answer for it: the report and what to run it with (the world has PHP 8.3.14 installed).
    assert_eq!(
        added.resolution.php.recommended.as_ref().unwrap().version,
        "8.3.14"
    );
    // It is on disk, with a schema.
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(b.file()).unwrap()).unwrap();
    assert_eq!(saved["schema"], 1);
    assert_eq!(saved["projects"][0]["id"], p.id.as_str());
    assert_eq!(b.store.list(), std::slice::from_ref(p));
    assert_eq!(b.store.get(&p.id).unwrap(), *p);
}

#[test]
fn adding_asks_for_an_absolute_folder_that_exists_and_not_twice() {
    let b = bench();
    let dir = b.project("shop", LARAVEL);
    assert_eq!(
        code(b.try_add(Path::new("shop"))),
        ErrorCode::InvalidPath,
        "relative"
    );
    // A relative path that DOES exist (tests run in the crate folder, which has src/) is still refused: where it points
    // depends on the folder the app happened to start in.
    assert!(Path::new("src").is_dir());
    let err = b.try_add(Path::new("src")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPath);
    assert!(err.message.contains("absolute"), "{}", err.message);
    assert_eq!(
        code(b.try_add(&b.tmp.path().join("nope"))),
        ErrorCode::InvalidPath,
        "missing"
    );
    assert_eq!(
        code(b.try_add(&dir.join("artisan"))),
        ErrorCode::InvalidPath,
        "a file"
    );
    assert!(b.store.list().is_empty(), "none of those added anything");

    let first = b.add(&dir);
    for spelling in [dir.clone(), dir.join("."), dir.join("public").join("..")] {
        let err = b.try_add(&spelling).unwrap_err();
        assert_eq!(err.code, ErrorCode::ProjectExists, "{}", spelling.display());
        assert_eq!(err.details.unwrap()["id"], first.project.id.as_str());
    }
    assert_eq!(b.store.list().len(), 1);
}

#[test]
fn every_project_gets_its_own_id() {
    let b = bench();
    let ids: Vec<String> = (0..20)
        .map(|n| b.add(&b.project(&format!("p{n}"), &[])).project.id)
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 20);
}

#[test]
fn nothing_is_ever_written_into_a_project_folder() {
    let b = bench();
    let dir = b.project("untouched", LARAVEL);
    let before = snapshot(&dir);
    let id = b.add(&dir).project.id;
    b.inspect(&id);
    b.update(
        &id,
        &ProjectPatch {
            name: Some("renamed".into()),
            ..ProjectPatch::default()
        },
    )
    .unwrap();
    b.update(&id, &use_php(BindingMode::Line, "8.1")).unwrap();
    b.update(
        &id,
        &ProjectPatch {
            doc_root: Some(DocRootChoice::Use {
                path: "public".into(),
            }),
            ..ProjectPatch::default()
        },
    )
    .unwrap();
    b.store.remove(&id).unwrap();
    assert_eq!(
        snapshot(&dir),
        before,
        "added, scanned, edited and removed: not one byte changed"
    );
}

// ------------------------------------------------------------------------------------------------ remove

#[test]
fn removing_forgets_the_project_and_leaves_its_folder_alone() {
    let b = bench();
    let dir = b.project("keep me", LARAVEL);
    let before = snapshot(&dir);
    let kept = b.add(&b.project("other", &[])).project;
    let id = b.add(&dir).project.id;
    b.store.remove(&id).unwrap();
    assert_eq!(snapshot(&dir), before);
    assert!(dir.join("artisan").is_file());
    assert_eq!(b.store.list(), [kept], "the other project stays");
    assert_eq!(b.store.get(&id).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(code(b.store.remove(&id)), ErrorCode::NotFound, "twice");
    // The folder can be added again afterwards, as a new project.
    assert_ne!(b.add(&dir).project.id, id);
}

// ------------------------------------------------------------------------------------------------ persistence

#[test]
fn the_list_survives_a_restart_and_a_damaged_file_starts_empty_without_losing_the_original() {
    let b = bench();
    let id = b.add(&b.project("persisted", LARAVEL)).project.id;
    b.update(&id, &use_php(BindingMode::Exact, "8.3.14"))
        .unwrap();
    let after = b.store.list();
    assert_eq!(
        ProjectStore::open(&b.home).unwrap().list(),
        after,
        "reopened"
    );

    fs::write(b.file(), "{ not json").unwrap();
    let recovered = ProjectStore::open(&b.home).unwrap();
    assert!(recovered.list().is_empty());
    let moved_aside = fs::read_dir(b.home.config_dir())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name().to_string_lossy().contains("projects.json")
                && e.file_name() != "projects.json"
        });
    assert!(moved_aside, "the damaged file is kept next to the new one");
}

#[test]
fn a_list_from_a_newer_lunarun_is_refused_and_left_untouched() {
    let b = bench();
    let text = r#"{"schema": 99, "projects": []}"#;
    fs::write(b.file(), text).unwrap();
    let err = ProjectStore::open(&b.home).err().unwrap();
    assert_eq!(err.code, ErrorCode::StateSchemaTooNew);
    assert_eq!(fs::read_to_string(b.file()).unwrap(), text);
}

// ------------------------------------------------------------------------------------------------ update

#[test]
fn a_name_is_trimmed_and_must_be_a_name() {
    let b = bench();
    let id = b.add(&b.project("named", &[])).project.id;
    let rename = |name: &str| {
        b.update(
            &id,
            &ProjectPatch {
                name: Some(name.into()),
                ..ProjectPatch::default()
            },
        )
    };
    assert_eq!(rename("  Shop ü  ").unwrap().name, "Shop ü");
    for bad in ["", "   ", "a\nb", &"x".repeat(101)] {
        let err = rename(bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
        assert_eq!(err.details.unwrap()["field"], "name");
    }
    assert_eq!(
        rename(&"x".repeat(100)).unwrap().name.len(),
        100,
        "100 is allowed"
    );
    assert_eq!(b.store.get(&id).unwrap().name.len(), 100);
}

#[test]
fn one_bad_part_of_a_patch_changes_nothing() {
    let b = bench();
    let id = b.add(&b.project("atomic", LARAVEL)).project.id;
    let before = b.store.get(&id).unwrap();
    let patch = ProjectPatch {
        name: Some("new name".into()),
        doc_root: Some(DocRootChoice::Use {
            path: "../elsewhere".into(),
        }),
        ..ProjectPatch::default()
    };
    assert_eq!(code(b.update(&id, &patch)), ErrorCode::InvalidInput);
    assert_eq!(b.store.get(&id).unwrap(), before);
    assert_eq!(
        code(b.update("nothing", &ProjectPatch::default())),
        ErrorCode::NotFound
    );
    // An empty patch is no change and no write.
    // (The time is kept in whole seconds, so a pointless write would often leave identical bytes: prove "no write" by
    // removing the file and seeing that it does not come back.)
    let saved = fs::read(b.file()).unwrap();
    assert_eq!(b.update(&id, &ProjectPatch::default()).unwrap(), before);
    assert_eq!(fs::read(b.file()).unwrap(), saved);
    fs::remove_file(b.file()).unwrap();
    assert_eq!(b.update(&id, &ProjectPatch::default()).unwrap(), before);
    assert!(!b.file().exists(), "an empty patch wrote the file");
}

#[test]
fn the_document_root_stays_inside_the_project_and_exists() {
    let b = bench();
    let dir = b.project(
        "roots",
        &[
            ("index.php", "<?php"),
            ("web/sub/index.php", ""),
            ("public_html/x", ""),
        ],
    );
    let id = b.add(&dir).project.id;
    assert_eq!(b.store.get(&id).unwrap().doc_root, ".");
    let set = |path: &str| {
        b.update(
            &id,
            &ProjectPatch {
                doc_root: Some(DocRootChoice::Use { path: path.into() }),
                ..ProjectPatch::default()
            },
        )
    };
    for (given, stored) in [
        ("web", "web"),
        (" web/ ", "web"),
        ("./web/sub", "web/sub"),
        (r"web\sub", "web/sub"),
        ("", "."),
        (".", "."),
    ] {
        let p = set(given).unwrap();
        assert_eq!(
            (p.doc_root.as_str(), p.doc_root_custom),
            (stored, true),
            "{given:?}"
        );
    }
    for bad in [
        "..",
        "../x",
        "web/../../x",
        "/etc",
        "/",
        r"C:\Windows",
        "c:",
        "missing",
        "index.php",
        "web\0",
    ] {
        let err = set(bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
        assert_eq!(err.details.unwrap()["field"], "docRoot");
    }
    assert_eq!(
        b.store.get(&id).unwrap().doc_root,
        ".",
        "none of the bad ones was applied"
    );
}

#[test]
fn a_chosen_document_root_survives_a_rescan_and_an_automatic_one_follows_the_folder() {
    let b = bench();
    let dir = b.project("follow", &[("index.php", "<?php"), ("web/index.php", "")]);
    let id = b.add(&dir).project.id;
    assert_eq!(b.store.get(&id).unwrap().doc_root, ".");
    // The project gains a public/index.php: an automatic root follows.
    fs::create_dir_all(dir.join("public")).unwrap();
    fs::write(dir.join("public/index.php"), "<?php").unwrap();
    assert_eq!(b.inspect(&id).project.doc_root, "public");
    // A chosen one stays, whatever the folder says now.
    b.update(
        &id,
        &ProjectPatch {
            doc_root: Some(DocRootChoice::Use { path: "web".into() }),
            ..ProjectPatch::default()
        },
    )
    .unwrap();
    assert_eq!(b.inspect(&id).project.doc_root, "web");
    // Back to automatic: what detection finds.
    let back = b
        .update(
            &id,
            &ProjectPatch {
                doc_root: Some(DocRootChoice::Auto),
                ..ProjectPatch::default()
            },
        )
        .unwrap();
    assert_eq!(
        (back.doc_root.as_str(), back.doc_root_custom),
        ("public", false)
    );
}

#[test]
fn bindings_are_versions_not_ranges_and_automatic_clears_them() {
    let b = bench();
    let id = b.add(&b.project("bound", LARAVEL)).project.id;
    for (mode, value) in [
        (BindingMode::Line, "8.3"),
        (BindingMode::Exact, "8.3.14"),
        (BindingMode::Line, "7.4"),
    ] {
        let p = b.update(&id, &use_php(mode, value)).unwrap();
        assert_eq!(
            p.bindings.php,
            Some(Binding {
                mode,
                value: value.into()
            })
        );
    }
    for (mode, value) in [
        (BindingMode::Line, "8"),
        (BindingMode::Line, "8.3.1"),
        (BindingMode::Line, ""),
        (BindingMode::Exact, "8.3"),
        (BindingMode::Exact, "8.3.14.1"),
        (BindingMode::Exact, "8.3.14-rc1"),
        (BindingMode::Exact, "^8.3"),
        (BindingMode::Line, "8.x"),
        (BindingMode::Line, "latest"),
        (BindingMode::Line, "8.3 "),
        (BindingMode::Line, "8..3"),
    ] {
        let err = b.update(&id, &use_php(mode, value)).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{mode:?} {value:?}");
        assert_eq!(err.details.unwrap()["field"], "php");
    }
    assert_eq!(
        b.store.get(&id).unwrap().bindings.php.unwrap().value,
        "7.4",
        "the last good one stays"
    );
    // Node's line is a major.
    assert!(b.update(&id, &use_node(BindingMode::Line, "22")).is_ok());
    assert!(
        b.update(&id, &use_node(BindingMode::Exact, "22.11.0"))
            .is_ok()
    );
    for (mode, value) in [
        (BindingMode::Line, "22.11"),
        (BindingMode::Line, "lts"),
        (BindingMode::Exact, "22"),
    ] {
        let err = b.update(&id, &use_node(mode, value)).unwrap_err();
        assert_eq!(
            (err.code, err.details.unwrap()["field"].clone()),
            (ErrorCode::InvalidInput, "node".into()),
            "{value:?}"
        );
    }
    let cleared = b
        .update(
            &id,
            &ProjectPatch {
                php: Some(BindingChoice::Auto),
                node: Some(BindingChoice::Auto),
                ..ProjectPatch::default()
            },
        )
        .unwrap();
    assert!(cleared.bindings.php.is_none() && cleared.bindings.node.is_none());
}

#[test]
fn a_binding_made_by_hand_decides_what_the_project_is_resolved_to() {
    let b = bench();
    let id = b.add(&b.project("manual", LARAVEL)).project.id;
    assert_eq!(b.inspect(&id).resolution.php.basis, Basis::Constraint);
    b.update(&id, &use_php(BindingMode::Line, "8.1")).unwrap();
    let chosen = b.inspect(&id).resolution;
    assert_eq!(
        chosen.php.basis,
        Basis::Manual,
        "even though the project asks for ^8.2"
    );
    assert_eq!(chosen.php.recommended.as_ref().unwrap().version, "8.1.30");
    b.update(
        &id,
        &ProjectPatch {
            php: Some(BindingChoice::Auto),
            ..ProjectPatch::default()
        },
    )
    .unwrap();
    assert_eq!(b.inspect(&id).resolution.php.basis, Basis::Constraint);
}

// ------------------------------------------------------------------------------------------------ inspect

#[test]
fn a_rescan_follows_the_files_and_keeps_the_hash_of_what_it_read() {
    let b = bench();
    let dir = b.project(
        "changes",
        &[
            ("composer.json", r#"{"require":{"php":"^8.1"}}"#),
            ("index.php", ""),
        ],
    );
    let first = b.add(&dir).project;
    assert!(first.framework.is_none());
    let same = b.inspect(&first.id).project;
    assert_eq!(
        same.last_detection.as_ref().unwrap().hash,
        first.last_detection.as_ref().unwrap().hash,
        "nothing changed"
    );

    fs::write(dir.join("artisan"), "").unwrap();
    fs::write(
        dir.join("composer.json"),
        r#"{"require":{"php":"^8.2","laravel/framework":"^11"}}"#,
    )
    .unwrap();
    let after = b.inspect(&first.id);
    assert!(
        after.project.framework.is_some(),
        "now it is a Laravel project"
    );
    assert_ne!(
        after.project.last_detection.as_ref().unwrap().hash,
        first.last_detection.as_ref().unwrap().hash
    );
    assert_eq!(
        b.store.get(&first.id).unwrap(),
        after.project,
        "and that is what is stored"
    );
}

#[test]
fn a_folder_that_has_gone_is_a_warning_and_the_stored_project_stays() {
    let b = bench();
    let dir = b.project("vanishing", LARAVEL);
    let stored = b.add(&dir).project;
    fs::remove_dir_all(&dir).unwrap();
    let after = b.inspect(&stored.id);
    assert!(
        after
            .report
            .warnings
            .iter()
            .any(|w| format!("{:?}", w.code) == "ProjectFolderMissing")
    );
    assert_eq!(after.project, stored, "what was known stays known");
    assert_eq!(b.store.get(&stored.id).unwrap(), stored);
    // It can still be renamed and removed.
    assert!(
        b.update(
            &stored.id,
            &ProjectPatch {
                name: Some("gone".into()),
                ..ProjectPatch::default()
            }
        )
        .is_ok()
    );
    assert!(b.store.remove(&stored.id).is_ok());
}

#[test]
fn inspecting_an_unknown_project_is_not_found() {
    let b = bench();
    let (installed, available) = (installed(Os::Windows), catalogs(Os::Windows));
    let around = Surroundings {
        installed: &installed,
        available: &available,
        os: Os::Windows,
    };
    assert_eq!(
        b.store.inspect("nope", &data(), &around).unwrap_err().code,
        ErrorCode::NotFound
    );
}

// ------------------------------------------------------------------------------------------------ bound_to

#[test]
fn a_version_is_bound_by_a_line_that_names_it_or_by_exact_version_and_not_by_what_is_automatic() {
    let b = bench();
    let line = b.add(&b.project("line", &[])).project.id;
    let exact = b.add(&b.project("exact", &[])).project.id;
    let node = b.add(&b.project("node", &[])).project.id;
    let auto = b.add(&b.project("auto", LARAVEL)).project.id;
    b.update(&line, &use_php(BindingMode::Line, "8.3")).unwrap();
    b.update(&exact, &use_php(BindingMode::Exact, "8.3.14"))
        .unwrap();
    b.update(&node, &use_node(BindingMode::Line, "22")).unwrap();
    let _ = auto;
    assert_eq!(b.store.bound_to("php", "8.3.14"), ["line", "exact"]);
    assert_eq!(
        b.store.bound_to("php", "8.3.9"),
        ["line"],
        "a line follows its patch versions"
    );
    assert!(b.store.bound_to("php", "8.1.30").is_empty());
    assert!(
        b.store.bound_to("php", "8.30.1").is_empty(),
        "8.30 is not 8.3"
    );
    assert_eq!(b.store.bound_to("node", "22.11.0"), ["node"]);
    assert!(b.store.bound_to("node", "2.1.0").is_empty());
    assert!(b.store.bound_to("mysql", "8.4.3").is_empty());
    assert!(b.store.bound_to("php", "garbage").is_empty());
}
