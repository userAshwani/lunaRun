//! Registry parsing, validation and platform selection against golden fixtures (FR-RT-01, FR-RT-07).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use luna_core::diagnostics::{Arch, Os};
use luna_core::error::ErrorCode;
use luna_core::registry::{ArtifactFormat, CalDate, Channel, Registry, SUPPORTED_SCHEMA};
use serde_json::{Value, json};

const VALID: &str = include_str!("fixtures/registry/valid.json");

fn valid() -> Registry {
    Registry::parse(VALID).expect("the valid fixture parses")
}

/// The valid fixture with `edit` applied to its JSON, to build one broken case at a time.
fn with(edit: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(VALID).unwrap();
    edit(&mut value);
    value.to_string()
}

fn first_artifact(value: &mut Value, version_index: usize) -> &mut Value {
    &mut value["runtimes"]["php"]["versions"][version_index]["artifacts"][0]
}

fn error_code(text: &str) -> ErrorCode {
    Registry::parse(text).expect_err("should be rejected").code
}

#[test]
fn parses_known_runtimes_and_ignores_unknown_fields() {
    let registry = valid();
    let ids: Vec<&str> = registry.runtime_ids().collect();
    assert_eq!(ids, ["node", "php"]);
    let php = registry.runtime("php").unwrap();
    assert_eq!(php.display_name, "PHP");
    assert_eq!(php.license.as_ref().unwrap().name, "PHP License v3.01");
}

#[test]
fn releases_are_newest_first_by_number_and_unknown_channels_are_dropped() {
    let registry = valid();
    let versions: Vec<&str> = registry
        .runtime("php")
        .unwrap()
        .releases
        .iter()
        .map(|r| r.version.as_str())
        .collect();
    // 9.0.0 has a channel this build does not know, so it is not offered at all.
    assert_eq!(
        versions,
        ["8.5.0-rc1", "8.4.0", "8.3.14", "8.3.9", "8.1.30"]
    );
    let rc = &registry.runtime("php").unwrap().releases[0];
    assert_eq!(rc.channel, Channel::Rc);
}

#[test]
fn unknown_os_arch_and_format_artifacts_are_skipped_not_errors() {
    let registry = valid();
    let release = registry
        .runtime("php")
        .unwrap()
        .releases
        .iter()
        .find(|r| r.version.as_str() == "8.3.14")
        .unwrap();
    // 4 usable artifacts; the freebsd, riscv64 and 7z ones are gone.
    assert_eq!(release.artifacts.len(), 4);
}

#[test]
fn artifact_fields_are_carried_over_and_the_hash_is_lowercased() {
    let registry = valid();
    let windows = registry
        .artifact_for("php", "8.3.9", Os::Windows, Arch::X86_64)
        .unwrap();
    assert!(!windows.rosetta_fallback);
    let artifact = windows.artifact;
    assert_eq!(artifact.format, ArtifactFormat::Zip);
    assert_eq!(artifact.variant.as_deref(), Some("nts-vs16"));
    assert_eq!(artifact.sha256, "a".repeat(64));
    assert_eq!(artifact.size, 31_000_000);
    assert_eq!(artifact.requires, ["vcredist-14-x64"]);

    let node = registry
        .artifact_for("node", "24.22.2", Os::Linux, Arch::X86_64)
        .unwrap();
    assert_eq!(node.artifact.strip_components, 1);
    assert_eq!(node.artifact.format, ArtifactFormat::TarXz);

    let mac = registry
        .artifact_for("php", "8.3.14", Os::Macos, Arch::X86_64)
        .unwrap();
    assert_eq!(mac.artifact.extensions, ["bcmath", "ctype"]);
}

#[test]
fn the_first_listed_build_wins_when_a_target_has_several_variants() {
    let registry = valid();
    // Two Windows builds exist for 8.3.14 with different variants; the registry lists NTS first.
    let choice = registry
        .artifact_for("php", "8.3.14", Os::Windows, Arch::X86_64)
        .unwrap();
    assert_eq!(choice.artifact.variant.as_deref(), Some("nts-vs16"));
}

#[test]
fn apple_silicon_prefers_native_and_falls_back_to_intel_only_when_needed() {
    let registry = valid();
    let native = registry
        .artifact_for("php", "8.4.0", Os::Macos, Arch::Aarch64)
        .unwrap();
    assert_eq!(native.artifact.arch, Arch::Aarch64);
    assert!(!native.rosetta_fallback);

    let fallback = registry
        .artifact_for("php", "8.3.14", Os::Macos, Arch::Aarch64)
        .unwrap();
    assert_eq!(fallback.artifact.arch, Arch::X86_64);
    assert!(fallback.rosetta_fallback);
}

#[test]
fn other_platforms_never_fall_back_to_another_architecture() {
    let registry = valid();
    for (os, arch) in [
        (Os::Windows, Arch::Aarch64),
        (Os::Linux, Arch::Aarch64),
        (Os::Macos, Arch::X86_64), // 8.1.30 has no artifacts at all
    ] {
        let version = if os == Os::Macos { "8.1.30" } else { "8.3.14" };
        let err = registry.artifact_for("php", version, os, arch).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::NoArtifactForPlatform,
            "{os:?}/{arch:?}"
        );
    }
    // Intel Macs do not get an Apple silicon build.
    let err = registry
        .artifact_for("php", "8.4.0", Os::Windows, Arch::X86_64)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NoArtifactForPlatform);
}

#[test]
fn asking_for_something_the_registry_does_not_have_is_not_found() {
    let registry = valid();
    for (runtime, version) in [("ruby", "3.3.0"), ("php", "8.3.99")] {
        let err = registry
            .artifact_for(runtime, version, Os::Linux, Arch::X86_64)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound, "{runtime} {version}");
    }
}

#[test]
fn available_versions_flag_eol_size_and_availability() {
    let registry = valid();
    let today = CalDate::new(2026, 10, 30).unwrap();
    let rows = registry.available_versions(Os::Windows, Arch::X86_64, today);
    let row = |runtime: &str, version: &str| {
        rows.iter()
            .find(|r| r.runtime == runtime && r.version == version)
            .unwrap_or_else(|| panic!("{runtime} {version}"))
    };

    // Nodes sort before PHP; inside PHP the newest comes first.
    assert_eq!(rows[0].runtime, "node");
    assert_eq!(rows[0].version, "24.22.2");

    let supported = row("php", "8.3.14");
    assert_eq!(supported.line, "8.3");
    assert!(!supported.eol);
    assert!(supported.available);
    assert_eq!(supported.size, Some(31_234_567));

    let ended = row("php", "8.1.30");
    assert_eq!(ended.line, "8.1");
    assert!(ended.eol, "8.1 ended 2025-12-31");
    assert!(!ended.available);
    assert_eq!(ended.size, None);

    assert!(!row("php", "8.4.0").available, "no Windows build of 8.4.0");
    assert_eq!(row("php", "8.5.0-rc1").channel, Channel::Rc);
    assert_eq!(
        row("php", "8.5.0-rc1").line,
        "8.5",
        "unlisted lines fall back to major.minor"
    );
    assert!(!row("php", "8.5.0-rc1").eol);
    assert!(
        !row("php", "8.4.0").eol,
        "a line with no eol date never ends"
    );

    let mac = registry.available_versions(Os::Macos, Arch::Aarch64, today);
    let rosetta = mac
        .iter()
        .find(|r| r.runtime == "php" && r.version == "8.3.14")
        .unwrap();
    assert!(rosetta.available && rosetta.rosetta_fallback);
}

#[test]
fn the_end_of_life_day_itself_counts_as_ended() {
    let registry = valid();
    let eol = |year, month, day| {
        registry
            .available_versions(
                Os::Windows,
                Arch::X86_64,
                CalDate::new(year, month, day).unwrap(),
            )
            .into_iter()
            .find(|r| r.runtime == "php" && r.version == "8.3.14")
            .unwrap()
            .eol
    };
    assert!(!eol(2027, 12, 30));
    assert!(eol(2027, 12, 31));
    assert!(eol(2028, 1, 1));
}

#[test]
fn a_newer_schema_is_refused_with_the_versions_in_the_details() {
    let err = Registry::parse(&with(|v| v["schema"] = json!(SUPPORTED_SCHEMA + 1))).unwrap_err();
    assert_eq!(err.code, ErrorCode::RegistryUnsupportedSchema);
    assert_eq!(err.details.unwrap()["supported"], SUPPORTED_SCHEMA);

    // Even when the rest of a future document no longer has today's shape.
    let future = json!({ "schema": 99, "runtimes": ["now", "a", "list"] }).to_string();
    assert_eq!(error_code(&future), ErrorCode::RegistryUnsupportedSchema);
}

#[test]
fn a_missing_or_bad_schema_or_garbage_is_invalid() {
    for text in [
        "".to_owned(),
        "not json".to_owned(),
        "[]".to_owned(),
        json!({ "runtimes": {} }).to_string(),
        json!({ "schema": 0, "runtimes": {} }).to_string(),
        json!({ "schema": "1", "runtimes": {} }).to_string(),
        json!({ "schema": 1, "runtimes": [] }).to_string(),
    ] {
        assert_eq!(error_code(&text), ErrorCode::RegistryInvalid, "{text}");
    }
}

#[test]
fn an_empty_registry_is_valid() {
    let registry = Registry::parse(&json!({ "schema": 1 }).to_string()).unwrap();
    assert_eq!(registry.runtime_ids().count(), 0);
}

#[test]
fn unsafe_artifact_data_is_rejected_with_the_culprit_named() {
    type Edit = Box<dyn Fn(&mut Value)>;
    let bad: Vec<(&str, Edit)> = vec![
        (
            "http url",
            Box::new(|v| first_artifact(v, 0)["url"] = json!("http://windows.php.net/a.zip")),
        ),
        (
            "file url",
            Box::new(|v| first_artifact(v, 0)["url"] = json!("file:///etc/passwd")),
        ),
        (
            "not a url",
            Box::new(|v| first_artifact(v, 0)["url"] = json!("php.zip")),
        ),
        (
            "short hash",
            Box::new(|v| first_artifact(v, 0)["sha256"] = json!("abc123")),
        ),
        (
            "non-hex hash",
            Box::new(|v| first_artifact(v, 0)["sha256"] = json!("g".repeat(64))),
        ),
        (
            "empty hash",
            Box::new(|v| first_artifact(v, 0)["sha256"] = json!("")),
        ),
        (
            "zero size",
            Box::new(|v| first_artifact(v, 0)["size"] = json!(0)),
        ),
        (
            "negative size",
            Box::new(|v| first_artifact(v, 0)["size"] = json!(-5)),
        ),
        (
            "missing url",
            Box::new(|v| {
                first_artifact(v, 0).as_object_mut().unwrap().remove("url");
            }),
        ),
    ];
    for (name, edit) in bad {
        let err = Registry::parse(&with(edit)).expect_err(name);
        assert_eq!(err.code, ErrorCode::RegistryInvalid, "{name}");
    }
    let err =
        Registry::parse(&with(|v| first_artifact(v, 0)["sha256"] = json!("abc"))).unwrap_err();
    assert!(
        err.message.contains("php 8.3.9"),
        "names the version: {}",
        err.message
    );
}

#[test]
fn names_that_become_folders_must_be_safe() {
    let rename_runtime = |name: &'static str| {
        with(move |v| {
            let php = v["runtimes"]["php"].take();
            v["runtimes"].as_object_mut().unwrap().remove("php");
            v["runtimes"][name] = php;
        })
    };
    for name in ["..", "a/b", "a\\b", "", "C:evil"] {
        assert_eq!(
            error_code(&rename_runtime(name)),
            ErrorCode::RegistryInvalid,
            "runtime {name:?}"
        );
    }
    for version in [
        "../8.3.1",
        "8.3.1/../..",
        "8.3",
        "8.3.1\\x",
        "latest",
        "8.3.1 ",
    ] {
        let text = with(|v| v["runtimes"]["php"]["versions"][0]["version"] = json!(version));
        let result = Registry::parse(&text);
        // "8.3" is a legal version shape, so it is the one input that parses.
        assert_eq!(result.is_ok(), version == "8.3", "version {version:?}");
    }
}

#[test]
fn duplicates_are_rejected() {
    let same_version = with(|v| {
        v["runtimes"]["php"]["versions"][1]["version"] = json!("8.3.9");
    });
    assert_eq!(error_code(&same_version), ErrorCode::RegistryInvalid);

    let same_target = with(|v| {
        let artifact = v["runtimes"]["php"]["versions"][0]["artifacts"][0].clone();
        v["runtimes"]["php"]["versions"][0]["artifacts"]
            .as_array_mut()
            .unwrap()
            .push(artifact);
    });
    assert_eq!(error_code(&same_target), ErrorCode::RegistryInvalid);
}

#[test]
fn bad_dates_in_lines_are_rejected() {
    for date in ["2027-02-30", "soon", "2027-12-31T00:00:00Z", "12/31/2027"] {
        let text = with(|v| v["runtimes"]["php"]["lines"]["8.3"]["eol"] = json!(date));
        assert_eq!(error_code(&text), ErrorCode::RegistryInvalid, "{date}");
    }
}

#[test]
fn the_longest_matching_line_wins() {
    let text = with(|v| {
        v["runtimes"]["node"]["lines"] = json!({
            "24": { "eol": "2028-04-30" },
            "24.4": { "eol": "2020-01-01" }
        });
    });
    let registry = Registry::parse(&text).unwrap();
    let today = CalDate::new(2026, 10, 30).unwrap();
    let rows = registry.available_versions(Os::Linux, Arch::X86_64, today);
    let row = |version: &str| rows.iter().find(|r| r.version == version).unwrap();
    assert_eq!(row("24.4.0").line, "24.4");
    assert!(row("24.4.0").eol);
    assert_eq!(row("24.22.2").line, "24");
    assert!(!row("24.22.2").eol);
}
