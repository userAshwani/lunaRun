//! The rows the Runtimes page shows: support dates and badges, community builds, and the grouping.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use luna_core::diagnostics::{Arch, Os};
use luna_core::registry::{AvailableVersion, CalDate, Registry, RuntimeCatalog};
use serde_json::{Value, json};

fn artifact(os: &str, arch: &str, variant: Option<&str>) -> Value {
    let mut value = json!({
        "os": os, "arch": arch, "format": "zip",
        "url": format!("https://example.com/{os}-{arch}-{}.zip", variant.unwrap_or("x")),
        "sha256": "0".repeat(64), "size": 1000,
    });
    if let Some(variant) = variant {
        value["variant"] = json!(variant);
    }
    value
}

fn registry() -> Registry {
    let text = json!({
        "schema": 1,
        "runtimes": {
            "php": {
                "displayName": "PHP",
                "license": { "name": "PHP License v3.01", "url": "https://www.php.net/license/" },
                "lines": {
                    "8.3": { "eol": "2027-12-31", "securityOnlySince": "2025-12-31" },
                    "8.1": { "eol": "2025-12-31" },
                    "8.4": {}
                },
                "versions": [
                    { "version": "8.4.0", "releasedAt": "2024-11-21",
                      "artifacts": [ artifact("windows", "x86_64", Some("nts-vs17")) ] },
                    { "version": "8.3.1", "artifacts": [
                        artifact("macos", "aarch64", Some("static-cli")),
                        artifact("macos", "aarch64", Some("static-fpm")),
                        artifact("windows", "x86_64", Some("nts-vs16")) ] },
                    { "version": "8.1.1", "artifacts": [ artifact("windows", "x86_64", None) ] }
                ]
            },
            "node": {
                "displayName": "Node.js",
                "versions": [ { "version": "24.0.0", "artifacts": [ artifact("linux", "x86_64", None) ] } ]
            }
        }
    })
    .to_string();
    Registry::parse(&text).unwrap()
}

fn on(today: &str) -> Vec<AvailableVersion> {
    registry().available_versions(Os::Windows, Arch::X86_64, CalDate::parse(today).unwrap())
}

fn row<'a>(rows: &'a [AvailableVersion], version: &str) -> &'a AvailableVersion {
    rows.iter().find(|r| r.version == version).unwrap()
}

#[test]
fn support_dates_and_badges_follow_the_calendar() {
    // Active support still runs: not security-only, not ended.
    let early = on("2025-06-01");
    let v83 = row(&early, "8.3.1");
    assert!(!v83.eol && !v83.security_only);
    assert_eq!(v83.eol_date.as_deref(), Some("2027-12-31"));

    // Active support ended on 2025-12-31: security fixes only, and the day itself counts.
    assert!(!row(&on("2025-12-30"), "8.3.1").security_only);
    assert!(row(&on("2025-12-31"), "8.3.1").security_only);
    let later = on("2026-11-03");
    assert!(row(&later, "8.3.1").security_only && !row(&later, "8.3.1").eol);

    // End of life: ended, and no longer "security only".
    let ended = on("2027-12-31");
    assert!(row(&ended, "8.3.1").eol && !row(&ended, "8.3.1").security_only);

    // A line with only an end date has no security-only phase.
    let old = row(&later, "8.1.1");
    assert!(old.eol && !old.security_only);
    assert_eq!(old.eol_date.as_deref(), Some("2025-12-31"));

    // A line the registry gives no dates for is never ended and shows no date.
    let current = row(&later, "8.4.0");
    assert!(!current.eol && !current.security_only);
    assert_eq!(current.eol_date, None);
}

#[test]
fn the_release_date_is_carried_when_the_registry_has_one() {
    let rows = on("2026-11-03");
    assert_eq!(
        row(&rows, "8.4.0").released_at.as_deref(),
        Some("2024-11-21")
    );
    assert_eq!(row(&rows, "8.3.1").released_at, None);
}

#[test]
fn static_php_is_marked_as_a_community_build_and_nothing_else_is() {
    let registry = registry();
    let today = CalDate::new(2026, 11, 3).unwrap();

    let mac = registry.available_versions(Os::Macos, Arch::Aarch64, today);
    assert!(row(&mac, "8.3.1").community, "static-cli plus static-fpm");
    assert!(row(&mac, "8.3.1").available);

    let windows = registry.available_versions(Os::Windows, Arch::X86_64, today);
    assert!(
        !row(&windows, "8.3.1").community,
        "the Windows zip comes from php.net with a checksum"
    );
    assert!(!row(&windows, "8.4.0").community);

    // Not available here: nothing to call a community build.
    assert!(!row(&mac, "8.4.0").available && !row(&mac, "8.4.0").community);
}

#[test]
fn the_catalog_groups_rows_by_runtime_with_name_and_license() {
    let today = CalDate::new(2026, 11, 3).unwrap();
    let catalog: Vec<RuntimeCatalog> = registry().catalog(Os::Windows, Arch::X86_64, today);

    let ids: Vec<&str> = catalog.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["node", "php"], "in name order");

    let php = &catalog[1];
    assert_eq!(php.display_name, "PHP");
    assert_eq!(
        php.license.as_ref().unwrap().url,
        "https://www.php.net/license/"
    );
    let versions: Vec<&str> = php.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(versions, ["8.4.0", "8.3.1", "8.1.1"], "newest first");
    assert!(php.versions.iter().all(|v| v.runtime == "php"));

    let node = &catalog[0];
    assert_eq!(node.license, None);
    assert!(
        !node.versions[0].available,
        "the only Node build is for Linux"
    );

    // The grouping is the same data as the flat list.
    let flat = registry().available_versions(Os::Windows, Arch::X86_64, today);
    let regrouped: Vec<AvailableVersion> = catalog.into_iter().flat_map(|c| c.versions).collect();
    assert_eq!(regrouped, flat);
}

#[test]
fn the_page_gets_camel_case_json() {
    let today = CalDate::new(2026, 11, 3).unwrap();
    let catalog = registry().catalog(Os::Windows, Arch::X86_64, today);
    let value = serde_json::to_value(&catalog[1]).unwrap();

    assert_eq!(value["displayName"], "PHP");
    let first = &value["versions"][1];
    assert_eq!(first["version"], "8.3.1");
    assert_eq!(first["eolDate"], "2027-12-31");
    assert_eq!(first["securityOnly"], true);
    assert_eq!(first["rosettaFallback"], false);
    assert_eq!(first["community"], false);
    assert!(first["releasedAt"].is_null());
}
