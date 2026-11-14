//! The rules of `resolve` one at a time (docs/PROJECT-DETECTION.md section 7; D-035), on small projects written for each
//! rule, in the Windows and Mac worlds of `projects_world`. The fixture golden files show the whole answer for realistic
//! projects; these show why.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod projects_world;

use std::fs;

use luna_core::diagnostics::Os;
use luna_core::projects::{
    Availability, Basis, BindingMode, DbNote, DetectionReport, IssueCode, Resolution, ResolvePrefs,
    detect, resolve,
};
use luna_core::registry::Channel;
use luna_core::runtimes::InstalledRuntime;
use projects_world::{catalogs, data, installed, row};
use tempfile::TempDir;

/// A project made of `files`, detected.
fn report(files: &[(&str, &str)]) -> DetectionReport {
    let dir = TempDir::new().unwrap();
    for (name, text) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    detect(dir.path(), &data())
}

fn php(constraint: &str) -> DetectionReport {
    report(&[(
        "composer.json",
        &format!("{{\"require\":{{\"php\":\"{constraint}\"}}}}"),
    )])
}

fn run(report: &DetectionReport, os: Os, prefs: impl FnOnce(&mut ResolvePrefs)) -> Resolution {
    let mut p = ResolvePrefs::new(os);
    prefs(&mut p);
    resolve(report, &installed(os), &catalogs(os), &data(), &p)
}

fn windows(report: &DetectionReport) -> Resolution {
    run(report, Os::Windows, |_| {})
}

fn version(resolution: &luna_core::projects::RuntimeResolution) -> String {
    let r = resolution
        .recommended
        .as_ref()
        .unwrap_or_else(|| panic!("no recommendation: {:?}", resolution.issues));
    format!(
        "{}{}",
        r.version,
        if r.installed { "" } else { " (install)" }
    )
}

#[test]
fn the_highest_installed_version_that_fits_is_recommended_before_anything_is_installed() {
    // 8.3.14 and 8.1.30 are installed, 8.5.12 exists: the installed one that fits wins, not the newest there is.
    let result = windows(&php("^8.1"));
    assert_eq!(version(&result.php), "8.3.14");
    assert_eq!(result.php.installed_matches, ["8.3.14", "8.1.30"]);
    assert_eq!(result.php.basis, Basis::Constraint);
    assert!(result.php.issues.is_empty());
}

#[test]
fn a_line_that_has_reached_its_end_of_life_is_used_only_when_nothing_better_fits() {
    let both = windows(&php(">=8.0"));
    assert_eq!(
        version(&both.php),
        "8.3.14",
        "8.1 is past its end of life, 8.3 is not"
    );
    assert!(!both.php.recommended.as_ref().unwrap().eol);
    let only_old = windows(&php("<8.2"));
    assert_eq!(version(&only_old.php), "8.1.30");
    assert!(
        only_old.php.recommended.as_ref().unwrap().eol,
        "it is used, and said to be end of life"
    );
}

#[test]
fn a_newer_line_at_its_end_of_life_loses_to_an_older_one_that_is_supported() {
    // Node's odd-numbered lines end early, so "newest" and "supported" can disagree. Here 8.3 is marked ended and 8.1 is not.
    let mut worlds = catalogs(Os::Windows);
    for row in &mut worlds.iter_mut().find(|c| c.id == "php").unwrap().versions {
        match row.line.as_str() {
            "8.3" => row.eol = true,
            "8.1" => row.eol = false,
            _ => {}
        }
    }
    let prefs = ResolvePrefs::new(Os::Windows);
    let with_installed = resolve(
        &php(">=8.0 <8.4"),
        &installed(Os::Windows),
        &worlds,
        &data(),
        &prefs,
    );
    assert_eq!(
        version(&with_installed.php),
        "8.1.30",
        "8.3.14 is installed and newer, but its line has ended"
    );
    assert!(!with_installed.php.recommended.as_ref().unwrap().eol);
    // Only the ended line fits: it is used, and said to be ended.
    let only_ended = resolve(
        &php("^8.3"),
        &installed(Os::Windows),
        &worlds,
        &data(),
        &prefs,
    );
    assert!(only_ended.php.recommended.as_ref().unwrap().eol);
    // The same for something to install.
    let nothing: Vec<InstalledRuntime> = Vec::new();
    let to_install = resolve(&php(">=8.0 <8.4"), &nothing, &worlds, &data(), &prefs);
    assert_eq!(
        version(&to_install.php),
        "8.2.35 (install)",
        "the newest supported line that fits, not 8.3"
    );
}

#[test]
fn nothing_installed_that_fits_means_the_newest_version_that_can_be_installed() {
    let result = windows(&php("^8.4"));
    let r = result.php.recommended.as_ref().unwrap();
    assert_eq!(
        (r.version.as_str(), r.installed, r.needs_install),
        ("8.5.12", false, true),
        "8.5.12, not 8.4.27: the newest that fits"
    );
    assert!(result.php.installed_matches.is_empty());
}

#[test]
fn the_users_default_line_decides_what_to_install_when_it_fits() {
    let nothing: Vec<InstalledRuntime> = Vec::new();
    let mut prefs = ResolvePrefs::new(Os::Windows);
    let without = resolve(
        &php(">=8.2"),
        &nothing,
        &catalogs(Os::Windows),
        &data(),
        &prefs,
    );
    assert_eq!(version(&without.php), "8.5.12 (install)");
    prefs.default_php_line = Some("8.3".to_owned());
    let with = resolve(
        &php(">=8.2"),
        &nothing,
        &catalogs(Os::Windows),
        &data(),
        &prefs,
    );
    assert_eq!(
        version(&with.php),
        "8.3.36 (install)",
        "the default line, since it satisfies the requirement and is supported"
    );
    // A default line that does not satisfy the requirement is ignored.
    prefs.default_php_line = Some("7.4".to_owned());
    assert_eq!(
        version(
            &resolve(
                &php(">=8.2"),
                &nothing,
                &catalogs(Os::Windows),
                &data(),
                &prefs
            )
            .php
        ),
        "8.5.12 (install)"
    );
}

#[test]
fn the_line_a_team_targets_beats_an_installed_version_of_another_line() {
    let targets_82 = report(&[(
        "composer.json",
        r#"{"require":{"php":"^8.2"},"config":{"platform":{"php":"8.2.0"}}}"#,
    )]);
    let result = windows(&targets_82);
    assert_eq!(
        version(&result.php),
        "8.2.35 (install)",
        "8.3.14 is installed and fits, but the team develops on 8.2"
    );
    assert_eq!(
        result.php.recommended.as_ref().unwrap().binding.value,
        "8.2"
    );

    // When the targeted line cannot fit the requirement, the target is not forced.
    let impossible = report(&[(
        "composer.json",
        r#"{"require":{"php":"^8.3"},"config":{"platform":{"php":"8.2.0"}}}"#,
    )]);
    assert_eq!(version(&windows(&impossible).php), "8.3.14");

    // A platform with only one number makes no target.
    let vague = report(&[(
        "composer.json",
        r#"{"require":{"php":"^8.2"},"config":{"platform":{"php":"8"}}}"#,
    )]);
    assert!(vague.php.as_ref().unwrap().preferred.is_none());
    assert!(
        vague
            .warnings
            .iter()
            .any(|w| format!("{:?}", w.code) == "InvalidConstraint")
    );
}

#[test]
fn a_binding_follows_the_line_unless_the_pin_is_a_whole_version() {
    let line = windows(&report(&[(".php-version", "8.3\n")]));
    let r = line.php.recommended.as_ref().unwrap();
    assert_eq!(
        (r.binding.mode, r.binding.value.as_str(), line.php.basis),
        (BindingMode::Line, "8.3", Basis::Pinned)
    );

    let exact = windows(&report(&[(".php-version", "8.3.14\n")]));
    let r = exact.php.recommended.as_ref().unwrap();
    assert_eq!(
        (r.binding.mode, r.binding.value.as_str()),
        (BindingMode::Exact, "8.3.14")
    );
    assert_eq!(version(&exact.php), "8.3.14");

    // An exact version that is not installed and that the registry no longer offers cannot be had.
    let gone = windows(&report(&[(".php-version", "8.3.2\n")]));
    assert!(gone.php.recommended.is_none());
    assert_eq!(gone.php.issues[0].code, IssueCode::NoCompatibleVersion);

    // A constraint, even one that names one version in a way, binds the line.
    let constraint = windows(&php("^8.1"));
    assert_eq!(
        constraint.php.recommended.as_ref().unwrap().binding.mode,
        BindingMode::Line
    );
}

#[test]
fn what_the_user_chose_for_this_project_wins_over_everything() {
    let project = report(&[
        ("composer.json", r#"{"require":{"php":"^8.3"}}"#),
        (".php-version", "8.3\n"),
    ]);
    let chosen = run(&project, Os::Windows, |p| {
        p.php_override = Some("8.1".to_owned())
    });
    assert_eq!(chosen.php.basis, Basis::Manual);
    assert_eq!(
        version(&chosen.php),
        "8.1.30",
        "even though the project says 8.3"
    );
    let exact = run(&project, Os::Windows, |p| {
        p.php_override = Some("8.3.14".to_owned())
    });
    assert_eq!(
        exact.php.recommended.as_ref().unwrap().binding.mode,
        BindingMode::Exact
    );
    // A choice that disagrees with the project says so (FR-PJ-04); one that agrees does not.
    assert_eq!(
        chosen.php.conflicts_with.as_deref(),
        Some(">=8.3.0 <8.4.0"),
        "8.1 against the .php-version pin of 8.3"
    );
    assert_eq!(exact.php.conflicts_with, None, "8.3.14 satisfies the pin");
    let plain = php("^8.3");
    let against_composer = run(&plain, Os::Windows, |p| {
        p.php_override = Some("8.1".to_owned())
    });
    assert_eq!(
        against_composer.php.conflicts_with.as_deref(),
        Some(">=8.3.0 <9.0.0")
    );
    let agreeing = run(&plain, Os::Windows, |p| {
        p.php_override = Some("8.3".to_owned())
    });
    assert_eq!(agreeing.php.conflicts_with, None);
    assert_eq!(
        windows(&plain).php.conflicts_with,
        None,
        "not a manual choice"
    );
    // Without any requirement there is nothing to disagree with.
    let free = run(&report(&[]), Os::Windows, |p| {
        p.php_override = Some("8.1".to_owned())
    });
    assert_eq!(free.php.conflicts_with, None);
    // An override that is not a version is ignored: the project's own requirement stands.
    let nonsense = run(&project, Os::Windows, |p| {
        p.php_override = Some("latest and greatest".to_owned())
    });
    assert_eq!(nonsense.php.basis, Basis::Pinned);
}

#[test]
fn a_project_that_says_nothing_gets_the_users_default_line_else_the_newest_installed() {
    let empty = report(&[]);
    let none = windows(&empty);
    assert_eq!(
        (none.php.basis, version(&none.php).as_str()),
        (Basis::NoRequirement, "8.3.14")
    );
    assert!(none.php.constraint.is_none());
    let default = run(&empty, Os::Windows, |p| {
        p.default_php_line = Some("8.1".to_owned())
    });
    assert_eq!(
        version(&default.php),
        "8.1.30",
        "the default line, though 8.3.14 is newer"
    );
    // A default line with nothing installed on it does not override what is installed.
    let missing = run(&empty, Os::Windows, |p| {
        p.default_php_line = Some("8.4".to_owned())
    });
    assert_eq!(version(&missing.php), "8.3.14");
    // Nothing installed at all: the newest supported version, to install.
    let fresh = resolve(
        &empty,
        &[],
        &catalogs(Os::Windows),
        &data(),
        &ResolvePrefs::new(Os::Windows),
    );
    assert_eq!(version(&fresh.php), "8.5.12 (install)");
}

#[test]
fn a_requirement_nothing_can_satisfy_is_an_answer_with_the_reason() {
    let empty = windows(&php(">=8.3 <8.2"));
    assert!(empty.php.recommended.is_none());
    assert_eq!(empty.php.issues[0].code, IssueCode::UnsatisfiableConstraint);

    let future = windows(&php("^9.0"));
    assert_eq!(future.php.issues[0].code, IssueCode::NoCompatibleVersion);
    assert!(future.php.issues[0].unavailable_lines.is_empty());
    assert!(
        future.php.issues[0]
            .message
            .contains("no PHP version known")
    );

    // On a Mac PHP 7.4 exists but has no download: the issue names the line and the system.
    let mac = run(&php("^7.4"), Os::Macos, |_| {});
    assert_eq!(mac.php.issues[0].unavailable_lines, ["7.4"]);
    assert!(
        mac.php.issues[0].message.contains("macOS"),
        "{}",
        mac.php.issues[0].message
    );
    let linux = run(&php("^7.4"), Os::Linux, |_| {});
    assert!(linux.php.issues[0].message.contains("Linux"));
}

#[test]
fn a_prerelease_is_offered_only_to_a_project_that_asks_for_one() {
    let mut worlds = catalogs(Os::Windows);
    let php_rows = &mut worlds.iter_mut().find(|c| c.id == "php").unwrap().versions;
    let mut rc = row("php", "8.6.0-rc1", false, true);
    rc.channel = Channel::Rc;
    php_rows.insert(0, rc);
    let nothing: Vec<InstalledRuntime> = Vec::new();
    let prefs = ResolvePrefs::new(Os::Windows);

    let plain = resolve(&php(">=8.2"), &nothing, &worlds, &data(), &prefs);
    assert_eq!(
        version(&plain.php),
        "8.5.12 (install)",
        "the release candidate is not offered"
    );
    let wants_rc = resolve(&php(">=8.6.0-rc1"), &nothing, &worlds, &data(), &prefs);
    assert_eq!(version(&wants_rc.php), "8.6.0-rc1 (install)");
}

#[test]
fn a_rosetta_download_is_flagged_so_the_page_can_say_it_is_slower() {
    let mut worlds = catalogs(Os::Macos);
    for catalog in &mut worlds {
        if catalog.id == "php" {
            for row in &mut catalog.versions {
                row.rosetta_fallback = true;
            }
        }
    }
    let nothing: Vec<InstalledRuntime> = Vec::new();
    let result = resolve(
        &php("^8.4"),
        &nothing,
        &worlds,
        &data(),
        &ResolvePrefs::new(Os::Macos),
    );
    assert!(result.php.recommended.as_ref().unwrap().rosetta_fallback);
}

#[test]
fn node_follows_its_own_pins_and_selectors() {
    // lts/* : the newest LTS line that is installed, not the newest LTS line there is.
    let lts = windows(&report(&[(".nvmrc", "lts/*\n")]));
    assert_eq!(version(lts.node.as_ref().unwrap()), "22.11.0");
    // Nothing installed: the newest LTS line to install (26 is LTS in the table and supported).
    let fresh = resolve(
        &report(&[(".nvmrc", "lts/*\n")]),
        &[],
        &catalogs(Os::Windows),
        &data(),
        &ResolvePrefs::new(Os::Windows),
    );
    assert_eq!(version(fresh.node.as_ref().unwrap()), "26.13.1 (install)");
    // `node` is the newest of what is installed.
    let latest = windows(&report(&[(".nvmrc", "node\n")]));
    assert_eq!(version(latest.node.as_ref().unwrap()), "22.11.0");
    // A codename is its line; an exact pin binds the exact version.
    let iron = windows(&report(&[(".nvmrc", "lts/iron\n")]));
    assert_eq!(version(iron.node.as_ref().unwrap()), "20.11.1");
    let exact = windows(&report(&[(".nvmrc", "v22.11.0\n")]));
    assert_eq!(
        exact
            .node
            .as_ref()
            .unwrap()
            .recommended
            .as_ref()
            .unwrap()
            .binding
            .mode,
        BindingMode::Exact
    );
    // No Node file at all: Node is not required.
    assert!(windows(&report(&[("composer.json", "{}")])).node.is_none());
}

#[test]
fn lts_means_a_long_term_support_line_even_when_a_newer_current_one_exists() {
    // 27 is newer than the newest LTS line (26) and not LTS itself: `lts/*` must skip it, `node` must not.
    let mut worlds = catalogs(Os::Windows);
    worlds
        .iter_mut()
        .find(|c| c.id == "node")
        .unwrap()
        .versions
        .insert(0, row("node", "27.0.0", false, true));
    let nothing: Vec<InstalledRuntime> = Vec::new();
    let prefs = ResolvePrefs::new(Os::Windows);
    let lts = resolve(
        &report(&[(".nvmrc", "lts/*\n")]),
        &nothing,
        &worlds,
        &data(),
        &prefs,
    );
    assert_eq!(version(lts.node.as_ref().unwrap()), "26.13.1 (install)");
    let latest = resolve(
        &report(&[(".nvmrc", "node\n")]),
        &nothing,
        &worlds,
        &data(),
        &prefs,
    );
    assert_eq!(version(latest.node.as_ref().unwrap()), "27.0.0 (install)");
    // With no usable requirement, only LTS lines are considered: a default is never a Current release.
    let mut unusable = ResolvePrefs::new(Os::Windows);
    unusable.node_override = Some("???".to_owned());
    let none = resolve(
        &report(&[("package.json", "{}")]),
        &nothing,
        &worlds,
        &data(),
        &unusable,
    );
    let node = none.node.as_ref().unwrap();
    assert_eq!(
        (node.basis, version(node).as_str()),
        (Basis::NoRequirement, "26.13.1 (install)")
    );
}

#[test]
fn an_override_for_node_works_without_any_node_file_and_a_bad_one_falls_back_to_lts() {
    let project = report(&[("composer.json", "{}")]);
    let chosen = run(&project, Os::Windows, |p| {
        p.node_override = Some("20".to_owned())
    });
    assert_eq!(chosen.node.as_ref().unwrap().basis, Basis::Manual);
    assert_eq!(version(chosen.node.as_ref().unwrap()), "20.11.1");
    let nonsense = run(&project, Os::Windows, |p| {
        p.node_override = Some("???".to_owned())
    });
    let node = nonsense.node.as_ref().unwrap();
    assert_eq!(node.basis, Basis::NoRequirement);
    assert_eq!(version(node), "22.11.0", "the newest installed LTS");
}

#[test]
fn the_database_suggestion_follows_the_engine_and_the_system() {
    let env = |connection: &str| report(&[(".env", &format!("DB_CONNECTION={connection}\n"))]);
    let suggest = |connection: &str, os: Os| run(&env(connection), os, |_| {}).database.unwrap();

    let mysql = suggest("mysql", Os::Windows);
    assert_eq!(
        (
            mysql.runtime.as_deref(),
            mysql.line.as_deref(),
            mysql.installed_version.as_deref()
        ),
        (Some("mysql"), Some("8.4"), Some("8.4.3"))
    );
    assert_eq!(
        suggest("mysql", Os::Macos).installed_version,
        None,
        "nothing installed there"
    );

    let maria = suggest("mariadb", Os::Linux);
    assert_eq!(
        (maria.runtime.as_deref(), maria.line.as_deref(), maria.note),
        (None, None, DbNote::None),
        "no MariaDB offered on this fake Linux"
    );
    let maria_win = suggest("mariadb", Os::Windows);
    assert_eq!(
        (maria_win.runtime.as_deref(), maria_win.line.as_deref()),
        (Some("mariadb"), Some("12.3"))
    );
    let maria_mac = suggest("mariadb", Os::Macos);
    assert_eq!(
        (
            maria_mac.runtime.as_deref(),
            maria_mac.line.as_deref(),
            maria_mac.note
        ),
        (Some("mysql"), Some("8.4"), DbNote::MariadbNotOnMacos)
    );

    assert_eq!(
        suggest("pgsql", Os::Windows).note,
        DbNote::PostgresNotSupported
    );
    assert_eq!(
        suggest("postgresql", Os::Windows).note,
        DbNote::PostgresNotSupported
    );
    assert_eq!(
        suggest("sqlite", Os::Windows).note,
        DbNote::SqliteNeedsNothing
    );
    assert_eq!(
        suggest("sqlsrv", Os::Windows).note,
        DbNote::SqlServerNotSupported
    );
    let other = suggest("oracle", Os::Windows);
    assert_eq!(
        (other.runtime, other.note),
        (None, DbNote::NoDatabaseSpecified)
    );
    assert!(
        run(&report(&[]), Os::Windows, |_| {}).database.is_none(),
        "no .env, no database"
    );
}

#[test]
fn extensions_are_judged_by_the_php_of_this_system() {
    let project = report(&[(
        "composer.json",
        r#"{"require":{"php":"^8.2","ext-intl":"*","ext-redis":"*","ext-mysqli":"*","ext-imaginary":"*","ext-zend-opcache":"*"}}"#,
    )]);
    let status = |os: Os, name: &str| {
        run(&project, os, |_| {})
            .extensions
            .into_iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .availability
    };
    assert_eq!(
        (status(Os::Windows, "intl"), status(Os::Macos, "intl")),
        (Availability::Enabled, Availability::Absent)
    );
    assert_eq!(
        (status(Os::Windows, "redis"), status(Os::Linux, "redis")),
        (Availability::Absent, Availability::Enabled)
    );
    assert_eq!(
        (status(Os::Windows, "mysqli"), status(Os::Macos, "mysqli")),
        (Availability::Enabled, Availability::Absent)
    );
    assert_eq!(
        (
            status(Os::Windows, "imaginary"),
            status(Os::Macos, "imaginary")
        ),
        (Availability::Unknown, Availability::Unknown)
    );
    let ext = project
        .php_extensions
        .iter()
        .find(|e| e.name == "zend-opcache")
        .unwrap();
    assert_eq!(
        (ext.ini_name.as_str(), ext.zend_extension),
        ("opcache", true)
    );
}
