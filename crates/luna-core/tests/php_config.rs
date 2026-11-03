//! The generated PHP configuration, against the real `php.ini-development` and extension list of PHP 8.3.36
//! for Windows. Every data folder has a space and non-ASCII characters in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use luna_core::LunaHome;
use luna_core::diagnostics::{Arch, Os};
use luna_core::error::ErrorCode;
use luna_core::runtimes::adapters::php_post_install;
use luna_core::runtimes::php_config::{
    WINDOWS_EXTENSIONS, available_extensions, render_static_ini, render_windows_ini, timezone_from,
};
use luna_core::runtimes::{
    CA_BUNDLE, InstallCtx, PhpSettings, php_config_paths, php_environment, write_php_config,
};
use tempfile::TempDir;

const BASE: &str = include_str!("fixtures/php/php.ini-development-8.3.36");
const EXTENSIONS: &str = include_str!("fixtures/php/ext-8.3.36.txt");

fn all_extensions() -> BTreeSet<String> {
    EXTENSIONS.lines().map(str::to_owned).collect()
}

fn home() -> (TempDir, LunaHome) {
    let tmp = TempDir::new().unwrap();
    let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
    (tmp, home)
}

fn settings<'a>(
    ext_dir: &'a Path,
    cafile: &'a Path,
    available: &'a BTreeSet<String>,
) -> PhpSettings<'a> {
    PhpSettings {
        extension_dir: ext_dir,
        cafile,
        timezone: "Europe/Berlin",
        available_extensions: available,
    }
}

fn windows_ext_dir() -> PathBuf {
    PathBuf::from(r"C:\Users\Ünï cödé\lüna home\runtimes\php\8.3.36\ext")
}

fn windows_cafile() -> PathBuf {
    PathBuf::from(r"C:\Users\Ünï cödé\lüna home\config\php\cacert.pem")
}

fn render(
    base: &str,
    available: &BTreeSet<String>,
) -> luna_core::runtimes::php_config::RenderedIni {
    render_windows_ini(
        base,
        &settings(&windows_ext_dir(), &windows_cafile(), available),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------------------------
// The Windows php.ini: only the intended lines change
// ---------------------------------------------------------------------------------------------

#[test]
fn only_the_intended_lines_of_php_ini_development_change() {
    let rendered = render(BASE, &all_extensions());

    let before: Vec<&str> = BASE.lines().collect();
    let after: Vec<&str> = rendered.text.lines().collect();
    assert_eq!(
        before.len(),
        after.len(),
        "nothing was appended: every setting had a line to replace"
    );

    let mut changed: Vec<String> = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a != b)
        .map(|(_, b)| (*b).to_owned())
        .collect();
    changed.sort();

    let ext_dir = windows_ext_dir();
    let cafile = windows_cafile();
    let mut expected: Vec<String> = vec![
        format!("extension_dir = \"{}\"", ext_dir.display()),
        "memory_limit = 512M".to_owned(),
        "upload_max_filesize = 128M".to_owned(),
        "post_max_size = 128M".to_owned(),
        "max_execution_time = 120".to_owned(),
        "date.timezone = \"Europe/Berlin\"".to_owned(),
        format!("curl.cainfo = \"{}\"", cafile.display()),
        format!("openssl.cafile = \"{}\"", cafile.display()),
        "opcache.enable = 1".to_owned(),
        "opcache.validate_timestamps = 1".to_owned(),
        "opcache.revalidate_freq = 0".to_owned(),
        "zend_extension=opcache".to_owned(),
    ];
    expected.extend(
        WINDOWS_EXTENSIONS
            .iter()
            .map(|name| format!("extension={name}")),
    );
    expected.sort();

    assert_eq!(changed, expected);
}

#[test]
fn extensions_are_switched_on_in_the_files_own_order_and_exif_stays_after_mbstring() {
    let rendered = render(BASE, &all_extensions());
    assert!(rendered.missing.is_empty(), "{:?}", rendered.missing);
    assert_eq!(rendered.enabled.len(), WINDOWS_EXTENSIONS.len() + 1);

    let position = |name: &str| rendered.enabled.iter().position(|e| e == name).unwrap();
    assert!(
        position("mbstring") < position("exif"),
        "exif needs mbstring first"
    );
    assert!(position("curl") < position("mbstring"));
    // Opcache is a zend_extension line that comes before the plain extensions in the file.
    let lines: Vec<&str> = rendered.text.lines().collect();
    let at = |needle: &str| lines.iter().position(|l| *l == needle).unwrap();
    assert!(at("extension=mbstring") < at("extension=exif"));
}

#[test]
fn an_extension_the_zip_does_not_have_is_reported_and_left_off() {
    let mut available = all_extensions();
    available.remove("intl");
    available.remove("sodium");
    available.remove("opcache");
    let rendered = render(BASE, &available);

    assert_eq!(rendered.missing, ["intl", "sodium", "opcache"]);
    assert!(!rendered.enabled.contains(&"intl".to_owned()));
    let lines: Vec<&str> = rendered.text.lines().collect();
    assert!(lines.contains(&";extension=intl"), "still commented out");
    assert!(lines.contains(&";extension=sodium"));
    assert!(lines.contains(&";zend_extension=opcache"));
    assert!(lines.contains(&"extension=curl"));
}

#[test]
fn line_endings_and_the_final_newline_are_kept() {
    assert!(
        BASE.contains("\r\n"),
        "the real file uses Windows line endings"
    );
    let rendered = render(BASE, &all_extensions());
    assert_eq!(
        rendered.text.matches('\n').count(),
        rendered.text.matches("\r\n").count(),
        "no bare line feed was introduced"
    );
    assert!(rendered.text.ends_with("\r\n"));

    let unix = BASE.replace("\r\n", "\n");
    let rendered = render(&unix, &all_extensions());
    assert!(!rendered.text.contains('\r'));
    let no_final = unix.trim_end().to_owned();
    assert!(!render(&no_final, &all_extensions()).text.ends_with('\n'));
}

#[test]
fn rendering_twice_gives_the_same_file() {
    let first = render(BASE, &all_extensions());
    let second = render(&first.text, &all_extensions());
    assert_eq!(first.text, second.text);
    assert_eq!(first.enabled, second.enabled);
}

#[test]
fn settings_missing_from_the_base_are_added_and_active_lines_beat_comments() {
    let base = "[PHP]\n; opcache.enable is switched on below\n; memory_limit = 1G is only an example\n; memory_limit = 64M\nmemory_limit = 128M\n\
                ; memory_limit_hint = 5\nopcache.enable_cli=0\n";
    let rendered = render(base, &BTreeSet::from(["curl".to_owned()]));
    let lines: Vec<&str> = rendered.text.lines().collect();

    // The active line was replaced, not a comment above it.
    assert!(lines.contains(&"; memory_limit = 64M"));
    assert!(lines.contains(&"memory_limit = 512M"));
    assert!(!lines.contains(&"memory_limit = 128M"));
    // A comment that merely starts with the key's name is prose, not a setting.
    assert!(lines.contains(&"; opcache.enable is switched on below"));
    // Longer names that merely start with the key are not the key.
    assert!(lines.contains(&"; memory_limit_hint = 5"));
    assert!(lines.contains(&"opcache.enable_cli=0"));
    assert!(
        lines.contains(&"opcache.enable = 1"),
        "added, not mistaken for opcache.enable_cli"
    );
    // A setting the base never mentioned goes at the end.
    assert!(lines.contains(&"upload_max_filesize = 128M"));
    assert_eq!(lines.last(), Some(&"extension=curl"));
}

#[test]
fn paths_are_quoted_and_those_php_would_misread_are_refused() {
    let rendered = render(BASE, &all_extensions());
    assert!(rendered.text.contains(
        "extension_dir = \"C:\\Users\\Ünï cödé\\lüna home\\runtimes\\php\\8.3.36\\ext\""
    ));

    let available = all_extensions();
    for bad in [
        "/data/with\"quote/ext",
        "/data/${HOME}/ext",
        "/data/line\nbreak/ext",
    ] {
        let err = render_windows_ini(
            BASE,
            &settings(Path::new(bad), &windows_cafile(), &available),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidPath, "{bad:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Static builds
// ---------------------------------------------------------------------------------------------

#[test]
fn a_static_build_gets_the_template_with_no_extension_lines() {
    let available = BTreeSet::new();
    let cafile =
        PathBuf::from("/Users/jörg/Library/Application Support/lunaRun/config/php/cacert.pem");
    let ini = render_static_ini(&settings(Path::new("/unused"), &cafile, &available)).unwrap();

    assert!(!ini.contains("{{"), "every placeholder was filled");
    assert!(ini.contains("date.timezone = \"Europe/Berlin\""));
    assert!(ini.contains(&format!("curl.cainfo = \"{}\"", cafile.display())));
    assert!(ini.contains(&format!("openssl.cafile = \"{}\"", cafile.display())));
    for line in [
        "memory_limit = 512M",
        "upload_max_filesize = 128M",
        "post_max_size = 128M",
        "max_execution_time = 120",
    ] {
        assert!(ini.lines().any(|l| l == line), "{line}");
    }
    assert!(
        !ini.lines().any(|l| l.starts_with("extension")),
        "static builds cannot load extensions"
    );
    assert!(!ini.contains("extension_dir"));
    assert!(
        !ini.contains("opcache"),
        "the static common build has no opcache"
    );
}

// ---------------------------------------------------------------------------------------------
// Time zone
// ---------------------------------------------------------------------------------------------

#[test]
fn the_time_zone_comes_from_the_localtime_link_then_etc_timezone_then_utc() {
    let zone = |link: Option<&str>, file: Option<&str>| timezone_from(link.map(Path::new), file);

    assert_eq!(
        zone(Some("/usr/share/zoneinfo/Europe/Berlin"), None),
        "Europe/Berlin"
    );
    assert_eq!(
        zone(
            Some("../usr/share/zoneinfo/America/Argentina/Buenos_Aires"),
            None
        ),
        "America/Argentina/Buenos_Aires"
    );
    assert_eq!(
        zone(Some("/var/db/timezone/zoneinfo/Asia/Kolkata"), None),
        "Asia/Kolkata"
    );
    assert_eq!(
        zone(Some("/usr/share/zoneinfo/Etc/GMT+5"), None),
        "Etc/GMT+5"
    );
    assert_eq!(zone(None, Some("Europe/Paris\n")), "Europe/Paris");
    assert_eq!(
        zone(Some("/usr/share/zoneinfo/Asia/Tokyo"), Some("Europe/Paris")),
        "Asia/Tokyo",
        "the link wins"
    );

    // Anything that is not a plain zone name falls back, so nothing odd lands inside a quoted value.
    for bad in [
        zone(Some("/usr/share/zoneinfo/../../etc/passwd"), None),
        zone(Some("/usr/share/zoneinfo/Europe/Ber lin"), None),
        zone(Some("/usr/share/zoneinfo/Europe/Berlin\"; evil"), None),
        zone(Some("/etc/localtime"), None),
        zone(None, Some("")),
        zone(None, Some("a b")),
        zone(None, Some("/absolute")),
        zone(None, Some(&"a".repeat(65))),
        zone(Some("/usr/share/zoneinfo/Europe/.."), None),
        zone(None, None),
    ] {
        assert_eq!(bad, "UTC");
    }
}

// ---------------------------------------------------------------------------------------------
// Files on disk
// ---------------------------------------------------------------------------------------------

#[test]
fn the_config_paths_and_environment_point_at_the_version_folder() {
    let (_tmp, home) = home();
    let paths = php_config_paths(&home, "8.3.36").unwrap();
    let php = home.config_dir().join("php");
    assert_eq!(paths.dir, php.join("8.3.36"));
    assert_eq!(paths.ini, php.join("8.3.36").join("php.ini"));
    assert_eq!(paths.conf_d, php.join("8.3.36").join("conf.d"));
    assert_eq!(
        paths.user_ini,
        php.join("8.3.36").join("conf.d").join("99-user.ini")
    );
    assert_eq!(
        paths.cacert,
        php.join("cacert.pem"),
        "one bundle shared by every version"
    );

    let env: BTreeMap<_, _> = php_environment(&home, "8.3.36")
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(env["PHPRC"], paths.dir.as_os_str());
    assert_eq!(env["PHP_INI_SCAN_DIR"], paths.conf_d.as_os_str());

    for bad in ["../x", "a/b", "", ".."] {
        assert!(php_config_paths(&home, bad).is_err(), "{bad:?}");
    }
}

#[test]
fn writing_the_config_creates_everything_and_never_overwrites_the_users_file() {
    let (_tmp, home) = home();
    let paths = php_config_paths(&home, "8.3.36").unwrap();

    write_php_config(&paths, "; first ini\n").unwrap();
    assert_eq!(fs::read_to_string(&paths.ini).unwrap(), "; first ini\n");
    assert_eq!(fs::read_to_string(&paths.cacert).unwrap(), CA_BUNDLE);
    assert!(paths.conf_d.is_dir());
    assert!(
        fs::read_to_string(&paths.user_ini)
            .unwrap()
            .contains("never overwrites")
    );

    // The user edits their file, and a damaged shared bundle is repaired by the next install.
    fs::write(&paths.user_ini, "memory_limit = 2G\n").unwrap();
    fs::write(&paths.cacert, "damaged").unwrap();
    write_php_config(&paths, "; second ini\n").unwrap();

    assert_eq!(
        fs::read_to_string(&paths.ini).unwrap(),
        "; second ini\n",
        "generated file is rewritten"
    );
    assert_eq!(
        fs::read_to_string(&paths.user_ini).unwrap(),
        "memory_limit = 2G\n",
        "the user's file is not"
    );
    assert_eq!(fs::read_to_string(&paths.cacert).unwrap(), CA_BUNDLE);
}

#[test]
fn the_ca_bundle_is_a_real_certificate_bundle() {
    let begins = CA_BUNDLE.matches("-----BEGIN CERTIFICATE-----").count();
    let ends = CA_BUNDLE.matches("-----END CERTIFICATE-----").count();
    assert_eq!(begins, ends);
    assert!(begins >= 100, "{begins} certificates");
    assert!(CA_BUNDLE.len() > 100_000);
    assert!(CA_BUNDLE.starts_with("##\n") || CA_BUNDLE.starts_with("##\r\n"));
}

#[test]
fn extensions_are_read_from_the_dll_names() {
    let tmp = TempDir::new().unwrap();
    for name in [
        "php_curl.dll",
        "php_mbstring.dll",
        "php_pdo_sqlite.dll",
        "readme.txt",
        "php_.dll",
        "curl.dll",
    ] {
        fs::write(tmp.path().join(name), "").unwrap();
    }
    let found = available_extensions(tmp.path());
    assert!(found.contains("curl") && found.contains("mbstring") && found.contains("pdo_sqlite"));
    assert!(!found.contains("readme") && !found.contains("curl.dll"));
    assert!(available_extensions(&tmp.path().join("missing")).is_empty());
}

// ---------------------------------------------------------------------------------------------
// post_install end to end
// ---------------------------------------------------------------------------------------------

/// A Windows-style staging folder: the zip's ini and one empty dll per extension.
fn windows_staging(root: &Path) -> PathBuf {
    let staging = root.join("php").join(".staging-job-1");
    fs::create_dir_all(staging.join("ext")).unwrap();
    fs::write(staging.join("php.ini-development"), BASE).unwrap();
    for name in all_extensions() {
        fs::write(staging.join("ext").join(format!("php_{name}.dll")), "").unwrap();
    }
    staging
}

fn ctx<'a>(dir: &'a Path, final_dir: &'a Path, home: &'a LunaHome, os: Os) -> InstallCtx<'a> {
    InstallCtx {
        dir,
        final_dir,
        home,
        os,
        arch: Arch::X86_64,
        version: "8.3.36",
    }
}

#[test]
fn a_windows_install_writes_an_ini_that_points_at_the_final_folder_not_the_staging_one() {
    let (_tmp, home) = home();
    let staging = windows_staging(&home.runtimes_dir());
    let final_dir = home.runtime_dir("php", "8.3.36").unwrap();

    php_post_install(
        &ctx(&staging, &final_dir, &home, Os::Windows),
        Path::new("unused"),
        "Asia/Kolkata",
    )
    .unwrap();

    let paths = php_config_paths(&home, "8.3.36").unwrap();
    let ini = fs::read_to_string(&paths.ini).unwrap();
    let ext_dir = final_dir.join("ext");
    assert!(
        ini.contains(&format!("extension_dir = \"{}\"", ext_dir.display())),
        "{ini}"
    );
    assert!(
        !ini.contains(".staging-job-1"),
        "the staging path must never leak into the ini"
    );
    assert!(ini.contains("date.timezone = \"Asia/Kolkata\""));
    assert!(ini.contains(&format!("curl.cainfo = \"{}\"", paths.cacert.display())));
    assert!(ini.contains("\nextension=curl\r\n") || ini.contains("\r\nextension=curl\r\n"));
    assert!(paths.cacert.is_file() && paths.conf_d.is_dir() && paths.user_ini.is_file());
}

#[test]
fn a_windows_download_without_php_ini_development_is_broken() {
    let (_tmp, home) = home();
    let staging = home.runtimes_dir().join("php").join(".staging-job-1");
    fs::create_dir_all(&staging).unwrap();
    let final_dir = home.runtime_dir("php", "8.3.36").unwrap();

    let err = php_post_install(
        &ctx(&staging, &final_dir, &home, Os::Windows),
        Path::new("unused"),
        "UTC",
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::RuntimeBroken);
    assert!(
        err.message.contains("php.ini-development"),
        "{}",
        err.message
    );
}

/// A stand-in for `codesign` that logs its arguments. `unsigned` makes `-v` fail, like an unsigned binary.
fn fake_codesign(dir: &Path, unsigned: bool, sign_fails: bool) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let verify_exit = i32::from(unsigned);
    let sign_exit = i32::from(sign_fails);
    if cfg!(windows) {
        let path = dir.join("codesign.cmd");
        fs::write(
            &path,
            format!(
                "@echo off\r\necho %*>>\"%~dp0calls.log\"\r\nif \"%1\"==\"-v\" exit /b {verify_exit}\r\nexit /b {sign_exit}\r\n"
            ),
        )
        .unwrap();
        path
    } else {
        let path = dir.join("codesign.sh");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls.log\"\n[ \"$1\" = \"-v\" ] && exit {verify_exit}\nexit {sign_exit}\n"
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

fn calls(tool: &Path) -> Vec<String> {
    // Read bytes: the Windows command prompt writes a non-ASCII path in the OEM code page, not UTF-8.
    let bytes = fs::read(tool.parent().unwrap().join("calls.log")).unwrap_or_default();
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(|l| l.trim().to_owned())
        .collect()
}

fn static_staging(root: &Path) -> PathBuf {
    let staging = root.join("php").join(".staging-job-2");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("php"), "binary").unwrap();
    fs::write(staging.join("php-fpm"), "binary").unwrap();
    staging
}

#[test]
fn a_macos_install_signs_each_unsigned_binary_and_leaves_signed_ones_alone() {
    let (tmp, home) = home();
    let staging = static_staging(&home.runtimes_dir());
    let final_dir = home.runtime_dir("php", "8.3.36").unwrap();

    let unsigned = fake_codesign(&tmp.path().join("tools unsigned"), true, false);
    php_post_install(
        &ctx(&staging, &final_dir, &home, Os::Macos),
        &unsigned,
        "UTC",
    )
    .unwrap();
    let log = calls(&unsigned);
    assert_eq!(
        log.len(),
        4,
        "check and sign for each of the two binaries: {log:?}"
    );
    // The home folder has non-ASCII characters that a Windows batch file logs in another code page, so
    // match on the ASCII end of the path: the staging folder and the binary's name.
    for name in ["php", "php-fpm"] {
        let tail = format!(".staging-job-2{}{name}", std::path::MAIN_SEPARATOR);
        assert!(
            log.iter().any(|l| l.starts_with("-v") && l.contains(&tail)),
            "{name}: {log:?}"
        );
        assert!(
            log.iter()
                .any(|l| l.starts_with("--force --sign -") && l.contains(&tail)),
            "{name}: {log:?}"
        );
    }

    let signed = fake_codesign(&tmp.path().join("tools signed"), false, false);
    php_post_install(&ctx(&staging, &final_dir, &home, Os::Macos), &signed, "UTC").unwrap();
    let log = calls(&signed);
    assert_eq!(log.len(), 2, "only the check: {log:?}");
    assert!(log.iter().all(|l| l.starts_with("-v")));
}

#[test]
fn a_failing_codesign_fails_the_install() {
    let (tmp, home) = home();
    let staging = static_staging(&home.runtimes_dir());
    let final_dir = home.runtime_dir("php", "8.3.36").unwrap();
    let broken = fake_codesign(&tmp.path().join("tools broken"), true, true);

    let err =
        php_post_install(&ctx(&staging, &final_dir, &home, Os::Macos), &broken, "UTC").unwrap_err();

    assert_eq!(err.code, ErrorCode::RuntimeBroken);
}

#[test]
fn linux_gets_the_static_ini_and_nothing_is_signed() {
    let (tmp, home) = home();
    let staging = static_staging(&home.runtimes_dir());
    let final_dir = home.runtime_dir("php", "8.3.36").unwrap();
    let tool = fake_codesign(&tmp.path().join("tools linux"), true, false);

    php_post_install(
        &ctx(&staging, &final_dir, &home, Os::Linux),
        &tool,
        "Europe/Berlin",
    )
    .unwrap();

    let ini = fs::read_to_string(php_config_paths(&home, "8.3.36").unwrap().ini).unwrap();
    assert!(ini.contains("date.timezone = \"Europe/Berlin\""));
    assert!(!ini.lines().any(|l| l.starts_with("extension")));
    assert!(calls(&tool).is_empty(), "codesign is for macOS only");
}

#[test]
fn macos_skips_a_binary_that_is_not_there() {
    let (tmp, home) = home();
    let staging = home.runtimes_dir().join("php").join(".staging-job-3");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("php"), "binary").unwrap(); // no php-fpm in this one
    let final_dir = home.runtime_dir("php", "8.3.36").unwrap();
    let tool = fake_codesign(&tmp.path().join("tools one"), true, false);

    php_post_install(&ctx(&staging, &final_dir, &home, Os::Macos), &tool, "UTC").unwrap();

    assert_eq!(calls(&tool).len(), 2, "check and sign for php only");
}
