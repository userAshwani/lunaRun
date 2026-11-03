//! The Visual C++ redistributable check (FR-RT-12): parsing `reg query`, the version floor, the refusal with
//! its details, and the rules for other systems and unknown requirements.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use luna_core::diagnostics::Os;
use luna_core::error::ErrorCode;
use luna_core::runtimes::prerequisites::{
    VCREDIST_DOWNLOAD_URL, VCREDIST_ID, VCREDIST_MINIMUM, VCREDIST_NAME, VcRedist,
    check_requirements, default_check, detect_vcredist, parse_reg_output,
};
use tempfile::TempDir;

/// The real output of `reg query` on a Windows 11 machine with the 2026 redistributable.
const REAL_OUTPUT: &str = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64\r\n    Version    REG_SZ    v14.51.36247.00\r\n    Installed    REG_DWORD    0x1\r\n    Major    REG_DWORD    0xe\r\n    Minor    REG_DWORD    0x33\r\n    Bld    REG_DWORD    0x8d97\r\n    Rbld    REG_DWORD    0x0\r\n\r\n";

fn requires(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| (*s).to_owned()).collect()
}

fn redist(installed: bool, major: u32, minor: u32, build: u32) -> VcRedist {
    VcRedist {
        installed,
        major,
        minor,
        build,
        version: None,
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing what `reg query` prints
// ---------------------------------------------------------------------------------------------

#[test]
fn the_real_output_is_read_by_value_name() {
    let found = parse_reg_output(REAL_OUTPUT).unwrap();
    assert_eq!(
        found,
        VcRedist {
            installed: true,
            major: 14,
            minor: 51,
            build: 36247,
            version: Some("v14.51.36247.00".to_owned()),
        }
    );
    assert_eq!(found.shown_version(), "14.51.36247.00");
    assert!(found.is_recent_enough());
}

#[test]
fn not_installed_and_unreadable_output_are_told_apart() {
    let zero = REAL_OUTPUT.replace(
        "Installed    REG_DWORD    0x1",
        "Installed    REG_DWORD    0x0",
    );
    assert!(!parse_reg_output(&zero).unwrap().installed);

    // No `Installed` value at all, an error in the user's language, nothing, noise.
    for text in [
        "",
        "garbage",
        "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64\r\n",
        "FEHLER: Das System kann den angegebenen Registrierungsschl\u{fc}ssel oder -wert nicht finden.",
        "    Major    REG_DWORD    0xe\r\n    Minor    REG_DWORD    0x33\r\n",
        // The right name with the wrong type is not the value we mean.
        "    Installed    REG_SZ    1\r\n",
        "    Installed    REG_DWORD\r\n",
    ] {
        assert_eq!(parse_reg_output(text), None, "{text:?}");
    }
}

#[test]
fn numbers_may_be_hex_or_decimal_and_the_version_text_is_optional() {
    let text = "    Installed    REG_DWORD    1\n    Major    REG_DWORD    14\n    Minor    REG_DWORD    0x28\n    Bld    REG_DWORD    33810\n";
    let found = parse_reg_output(text).unwrap();
    assert_eq!((found.major, found.minor, found.build), (14, 40, 33810));
    assert_eq!(found.version, None);
    assert_eq!(
        found.shown_version(),
        "14.40.33810",
        "built from the numbers"
    );

    // Upper-case hex and a stray value we do not know.
    let text = "    Installed    REG_DWORD    0X1\n    Mystery    REG_DWORD    0x9\n    Major    REG_DWORD    0XE\n";
    let found = parse_reg_output(text).unwrap();
    assert!(found.installed);
    assert_eq!(found.major, 14);
}

#[test]
fn the_version_floor_is_14_40() {
    assert_eq!(VCREDIST_MINIMUM, (14, 40));
    let ok = |major, minor| redist(true, major, minor, 0).is_recent_enough();
    assert!(!ok(14, 0), "the 2015 runtime");
    assert!(!ok(14, 29), "the 2019 runtime");
    assert!(!ok(14, 39), "one below the floor");
    assert!(ok(14, 40), "the floor itself");
    assert!(ok(14, 51));
    assert!(ok(15, 0));
    assert!(!ok(13, 99));
}

// ---------------------------------------------------------------------------------------------
// The requirement check
// ---------------------------------------------------------------------------------------------

#[test]
fn a_recent_installed_redistributable_passes_and_is_asked_about_once_per_requirement() {
    let asked = Cell::new(0);
    let detect = || {
        asked.set(asked.get() + 1);
        Ok(Some(redist(true, 14, 51, 36247)))
    };
    check_requirements(Os::Windows, &requires(&[VCREDIST_ID]), &detect).unwrap();
    assert_eq!(asked.get(), 1);
}

#[test]
fn a_missing_redistributable_is_refused_with_everything_the_ui_needs() {
    let err = check_requirements(Os::Windows, &requires(&[VCREDIST_ID]), &|| Ok(None)).unwrap_err();

    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert!(err.message.contains(VCREDIST_NAME), "{}", err.message);
    let details = err.details.unwrap();
    assert_eq!(details["requirement"], VCREDIST_ID);
    assert_eq!(details["name"], VCREDIST_NAME);
    assert_eq!(details["downloadUrl"], VCREDIST_DOWNLOAD_URL);
    assert_eq!(details["minimum"], "14.40");
    assert!(details["found"].is_null(), "nothing was found");
}

#[test]
fn a_flag_of_zero_counts_as_not_installed() {
    let detect = || Ok(Some(redist(false, 14, 51, 36247)));
    let err = check_requirements(Os::Windows, &requires(&[VCREDIST_ID]), &detect).unwrap_err();
    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert!(err.message.contains("not installed"));
}

#[test]
fn an_old_redistributable_is_refused_and_says_which_version_it_found() {
    let detect = || Ok(Some(redist(true, 14, 29, 30133)));
    let err = check_requirements(Os::Windows, &requires(&[VCREDIST_ID]), &detect).unwrap_err();

    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert!(err.message.contains("14.29.30133"), "{}", err.message);
    assert!(err.message.contains("14.40"), "{}", err.message);
    let details = err.details.unwrap();
    assert_eq!(details["found"], "14.29.30133");
    assert_eq!(details["downloadUrl"], VCREDIST_DOWNLOAD_URL);
}

#[test]
fn an_unknown_requirement_is_refused_not_ignored() {
    let detect = || panic!("nothing to detect for an unknown requirement");
    let err = check_requirements(Os::Windows, &requires(&["dotnet-9-x64"]), &detect).unwrap_err();
    assert_eq!(err.code, ErrorCode::PrerequisiteMissing);
    assert!(err.message.contains("dotnet-9-x64"), "{}", err.message);
    assert_eq!(err.details.unwrap()["requirement"], "dotnet-9-x64");

    // One unknown among known ones is still refused.
    let known = || Ok(Some(redist(true, 14, 51, 36247)));
    assert!(
        check_requirements(
            Os::Windows,
            &requires(&[VCREDIST_ID, "dotnet-9-x64"]),
            &known
        )
        .is_err()
    );
}

#[test]
fn nothing_is_checked_when_nothing_is_required_or_on_other_systems() {
    let never = || panic!("the registry must not be read");
    check_requirements(Os::Windows, &[], &never).unwrap();
    for os in [Os::Macos, Os::Linux, Os::Unknown] {
        check_requirements(os, &requires(&[VCREDIST_ID]), &never).unwrap();
        check_requirements(os, &requires(&["dotnet-9-x64"]), &never).unwrap();
        default_check(os, &requires(&[VCREDIST_ID]), Path::new("/no/reg")).unwrap();
    }
}

#[test]
fn when_the_registry_cannot_be_read_the_install_is_not_blocked() {
    let broken = || {
        Err(luna_core::LunaError::new(
            ErrorCode::RuntimeBroken,
            "reg.exe could not be started",
        ))
    };
    check_requirements(Os::Windows, &requires(&[VCREDIST_ID]), &broken).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Asking `reg`, through a stand-in program
// ---------------------------------------------------------------------------------------------

/// A stand-in for `reg.exe` that logs its arguments and prints `output`, then exits with `code`.
fn fake_reg(dir: &Path, output: &str, code: i32) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    if cfg!(windows) {
        let path = dir.join("reg.cmd");
        let echoes: String = output.lines().map(|l| format!("echo {l}\r\n")).collect();
        fs::write(
            &path,
            format!("@echo off\r\necho %*>>\"%~dp0calls.log\"\r\n{echoes}exit /b {code}\r\n"),
        )
        .unwrap();
        path
    } else {
        let path = dir.join("reg.sh");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls.log\"\ncat <<'EOF'\n{output}\nEOF\nexit {code}\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }
}

#[test]
fn the_registry_key_is_asked_for_by_argument_list_and_the_answer_is_parsed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("fake système");
    let reg = fake_reg(&dir, REAL_OUTPUT.trim(), 0);

    let found = detect_vcredist(&reg).unwrap().unwrap();

    assert!(found.installed && found.is_recent_enough());
    assert_eq!(found.shown_version(), "14.51.36247.00");
    let log = fs::read_to_string(dir.join("calls.log")).unwrap();
    assert!(log.contains("query"), "{log}");
    assert!(
        log.contains(r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64"),
        "the 64-bit runtime key: {log}"
    );
}

#[test]
fn a_key_that_does_not_exist_is_not_installed_whatever_language_the_error_is_in() {
    let tmp = TempDir::new().unwrap();
    let reg = fake_reg(
        &tmp.path().join("fake"),
        "FEHLER: Das System kann den angegebenen Registrierungsschlüssel nicht finden.",
        1,
    );
    assert_eq!(detect_vcredist(&reg).unwrap(), None);
}

#[test]
fn a_failing_reg_is_never_read_for_values() {
    // A failure exit means "not there", even if the program printed something that looks like a value.
    let tmp = TempDir::new().unwrap();
    let reg = fake_reg(&tmp.path().join("fake"), REAL_OUTPUT.trim(), 1);
    assert_eq!(detect_vcredist(&reg).unwrap(), None);
}

#[test]
fn a_reg_that_cannot_be_started_is_an_error_not_a_verdict() {
    let tmp = TempDir::new().unwrap();
    let err = detect_vcredist(&tmp.path().join("no such reg.exe")).unwrap_err();
    assert_eq!(err.code, ErrorCode::RuntimeBroken);
}

#[cfg(windows)]
#[test]
fn the_real_reg_query_runs_and_is_understood_on_this_machine() {
    // Read-only: it asks Windows about a registry key and changes nothing.
    let reg = luna_core::runtimes::prerequisites::default_reg_path();
    assert!(reg.is_file(), "{}", reg.display());

    let answer = detect_vcredist(&reg).expect("reg.exe runs");

    // A machine with Visual Studio or any recent app has it, a bare one does not: both are valid answers.
    if let Some(found) = answer
        && found.installed
    {
        assert_eq!(
            found.major, 14,
            "the 2015-2022 runtimes share the 14.x line"
        );
    }
}
