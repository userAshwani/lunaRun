//! The "Laravel starter pack" of the first-run wizard (FR-AP-06, D-044), chosen from the registry the app ships. The tests
//! check the rules, never a version number: the registry is refreshed every day (TRACKER trap 13).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use luna_core::diagnostics::{Arch, Os};
use luna_core::onboarding::{STARTER_MYSQL_LINE, starter_pack};
use luna_core::projects::DetectionData;
use luna_core::registry::{CalDate, Channel, RegistryHolder, RuntimeCatalog, RuntimeVersion};
use luna_core::runtimes::{InstallReceipt, InstalledRuntime};

fn catalog(os: Os, arch: Arch) -> Vec<RuntimeCatalog> {
    let today = CalDate::new(2026, 11, 27).unwrap();
    RegistryHolder::with_bundled()
        .unwrap()
        .available(os, arch, today)
        .runtimes
}

fn installed(runtime: &str, version: &str) -> InstalledRuntime {
    InstalledRuntime {
        runtime: runtime.to_owned(),
        version: version.to_owned(),
        dir: PathBuf::from("/x"),
        receipt: InstallReceipt::new(runtime, version, Os::Windows, Arch::X86_64, false, &[]),
    }
}

/// The first version of `runtime` that is offered here, stable and supported, and that `fits`.
fn first(catalog: &[RuntimeCatalog], runtime: &str, fits: impl Fn(&str) -> bool) -> Option<String> {
    catalog
        .iter()
        .find(|c| c.id == runtime)?
        .versions
        .iter()
        .find(|v| v.available && v.channel == Channel::Stable && !v.eol && fits(&v.version))
        .map(|v| v.version.clone())
}

#[test]
fn the_pack_is_the_newest_supported_php_composer_node_lts_and_mysql_8_4() {
    let data = DetectionData::bundled().unwrap();
    let lts = data.lts_majors();
    let catalog = catalog(Os::Windows, Arch::X86_64);

    let pack = starter_pack(&catalog, &[], &data);

    let runtimes: Vec<&str> = pack.items.iter().map(|i| i.runtime.as_str()).collect();
    assert_eq!(runtimes, ["php", "composer", "node", "mysql"]);
    assert!(pack.unavailable.is_empty(), "{:?}", pack.unavailable);
    let version = |runtime: &str| {
        pack.items
            .iter()
            .find(|i| i.runtime == runtime)
            .unwrap()
            .version
            .clone()
    };
    assert_eq!(Some(version("php")), first(&catalog, "php", |_| true));
    assert_eq!(
        Some(version("composer")),
        first(&catalog, "composer", |_| true)
    );
    let node = version("node");
    let node_major: u64 = node.split('.').next().unwrap().parse().unwrap();
    assert!(
        lts.contains(&node_major),
        "Node {node} is not an LTS line ({lts:?})"
    );
    assert_eq!(
        Some(node.clone()),
        first(&catalog, "node", |v| v
            .split('.')
            .next()
            .and_then(|m| m.parse::<u64>().ok())
            .is_some_and(|m| lts.contains(&m)))
    );
    let mysql = version("mysql");
    assert!(
        RuntimeVersion::parse(&mysql)
            .unwrap()
            .is_on_line(STARTER_MYSQL_LINE),
        "{mysql}"
    );
    assert_eq!(
        STARTER_MYSQL_LINE, "8.4",
        "the line project detection suggests"
    );
    // Nothing is installed: everything is downloaded, and the total is the sum.
    assert!(pack.items.iter().all(|i| i.installed_version.is_none()));
    let sum: u64 = pack.items.iter().filter_map(|i| i.size).sum();
    assert_eq!(pack.download_bytes(), sum);
    assert!(sum > 0);
}

#[test]
fn what_is_installed_already_is_not_downloaded_again() {
    let data = DetectionData::bundled().unwrap();
    let catalog = catalog(Os::Windows, Arch::X86_64);
    let fresh = starter_pack(&catalog, &[], &data);
    let php = fresh.items.iter().find(|i| i.runtime == "php").unwrap();
    // An older patch of the same line counts as installed; another line does not.
    let older_patch = format!("{}.0", php.line);
    let pack = starter_pack(
        &catalog,
        &[installed("php", &older_patch), installed("mysql", "9.7.1")],
        &data,
    );
    let item = |runtime: &str| pack.items.iter().find(|i| i.runtime == runtime).unwrap();
    assert_eq!(
        item("php").installed_version.as_deref(),
        Some(older_patch.as_str())
    );
    assert_eq!(
        item("mysql").installed_version,
        None,
        "9.7 is not the 8.4 line"
    );
    assert_eq!(
        pack.download_bytes(),
        fresh.download_bytes() - php.size.unwrap_or(0)
    );
}

#[test]
fn what_has_no_build_for_this_computer_is_named_not_guessed() {
    // MySQL has no Linux ARM build in the registry (D-017): the pack says so instead of offering another line.
    let data = DetectionData::bundled().unwrap();
    let pack = starter_pack(&catalog(Os::Linux, Arch::Aarch64), &[], &data);
    assert!(pack.unavailable.contains(&"mysql".to_owned()), "{pack:?}");
    assert!(!pack.items.iter().any(|i| i.runtime == "mysql"));
    assert!(pack.items.iter().any(|i| i.runtime == "php"));
}

#[test]
fn a_release_candidate_or_an_ended_line_is_never_in_the_pack() {
    let data = DetectionData::bundled().unwrap();
    let mut catalog = catalog(Os::Windows, Arch::X86_64);
    let php = catalog.iter_mut().find(|c| c.id == "php").unwrap();
    let mut rc = php.versions[0].clone();
    rc.version = "9.0.0-rc1".to_owned();
    rc.channel = Channel::Rc;
    let mut ended = php.versions[0].clone();
    ended.version = "8.9.9".to_owned();
    ended.eol = true;
    php.versions.insert(0, ended);
    php.versions.insert(0, rc);

    let pack = starter_pack(&catalog, &[], &data);
    let chosen = &pack
        .items
        .iter()
        .find(|i| i.runtime == "php")
        .unwrap()
        .version;
    assert_ne!(chosen, "9.0.0-rc1");
    assert_ne!(chosen, "8.9.9");
}

#[test]
fn a_newer_node_that_is_not_lts_is_never_in_the_pack() {
    // The newest Node.js in the list can itself be an LTS line, which would hide a pack that ignores LTS.
    let data = DetectionData::bundled().unwrap();
    let mut catalog = catalog(Os::Windows, Arch::X86_64);
    let node = catalog.iter_mut().find(|c| c.id == "node").unwrap();
    let not_lts_major = data.lts_majors()[0] + 1;
    let mut current = node.versions[0].clone();
    current.version = format!("{not_lts_major}.0.0");
    current.line = not_lts_major.to_string();
    node.versions.insert(0, current);

    let pack = starter_pack(&catalog, &[], &data);
    let chosen = &pack
        .items
        .iter()
        .find(|i| i.runtime == "node")
        .unwrap()
        .version;
    assert_ne!(*chosen, format!("{not_lts_major}.0.0"));
}
