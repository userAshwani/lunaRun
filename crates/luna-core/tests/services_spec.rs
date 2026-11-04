//! Service specs are checked before anything starts: a program runs only from inside `LUNA_HOME` (golden rule 6).
//! Every data folder has a space and a non-ASCII letter in its path (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use luna_core::services::{HelperCommand, ReadyCheck, ServiceSpec, StopMethod};
use luna_core::{ErrorCode, LunaHome};
use tempfile::TempDir;

struct Bench {
    _tmp: TempDir,
    home: LunaHome,
    /// A folder next to the data folder, not inside it.
    outside: PathBuf,
}

impl Bench {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        home.ensure_layout().unwrap(); // as the app does at startup
        let outside = tmp.path().join("somewhere else");
        fs::create_dir_all(&outside).unwrap();
        Self {
            _tmp: tmp,
            home,
            outside,
        }
    }

    /// A program-like file `runtimes/<name>/1.0/bin/<name>` inside the data folder.
    fn program(&self, name: &str) -> PathBuf {
        let dir = self.home.runtimes_dir().join(name).join("1.0").join("bin");
        fs::create_dir_all(&dir).unwrap();
        make_program(&dir.join(name))
    }

    fn spec(&self) -> ServiceSpec {
        ServiceSpec::new("caddy", self.program("caddy"), 8101)
    }
}

fn make_program(path: &Path) -> PathBuf {
    fs::write(path, "stand-in").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path.to_path_buf()
}

fn code(spec: &ServiceSpec, bench: &Bench) -> ErrorCode {
    spec.validate(&bench.home).unwrap_err().code
}

#[test]
fn a_program_inside_the_data_folder_is_accepted() {
    let bench = Bench::new();
    bench.spec().validate(&bench.home).unwrap();
}

#[test]
fn a_program_outside_the_data_folder_is_refused() {
    let bench = Bench::new();
    let mut spec = bench.spec();
    spec.program = make_program(&bench.outside.join("evil"));

    let error = spec.validate(&bench.home).unwrap_err();

    assert_eq!(error.code, ErrorCode::PathOutsideRoot);
    assert!(
        error
            .message
            .contains("not inside the lunaRun data directory")
    );
}

#[test]
fn dot_dot_cannot_walk_out_of_the_data_folder() {
    let bench = Bench::new();
    let outside = make_program(&bench.outside.join("evil"));
    let mut spec = bench.spec();
    // Starts inside the data folder, resolves to a file next to it.
    spec.program = bench
        .home
        .root()
        .join("runtimes")
        .join("..")
        .join("..")
        .join("somewhere else")
        .join("evil");
    assert!(
        spec.program.exists()
            && fs::canonicalize(&spec.program).unwrap() == fs::canonicalize(outside).unwrap()
    );

    assert_eq!(code(&spec, &bench), ErrorCode::PathOutsideRoot);
}

#[cfg(unix)]
#[test]
fn a_link_inside_the_data_folder_to_a_program_outside_is_refused() {
    let bench = Bench::new();
    let outside = make_program(&bench.outside.join("evil"));
    let link = bench.home.runtimes_dir().join("sneaky");
    std::os::unix::fs::symlink(outside, &link).unwrap();
    let mut spec = bench.spec();
    spec.program = link;

    assert_eq!(code(&spec, &bench), ErrorCode::PathOutsideRoot);
}

#[cfg(unix)]
#[test]
fn a_file_that_is_not_executable_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let bench = Bench::new();
    let mut spec = bench.spec();
    fs::set_permissions(&spec.program, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(code(&spec, &bench), ErrorCode::PermissionDenied);

    spec.program = bench.program("other");
    spec.validate(&bench.home).unwrap();
}

#[test]
fn the_data_folder_itself_and_folders_are_not_programs() {
    let bench = Bench::new();
    let mut spec = bench.spec();

    spec.program = bench.home.root().to_path_buf();
    assert_eq!(code(&spec, &bench), ErrorCode::PathOutsideRoot);

    spec.program = bench.home.runtimes_dir();
    assert_eq!(code(&spec, &bench), ErrorCode::InvalidPath);
}

#[test]
fn a_relative_or_missing_program_is_refused() {
    let bench = Bench::new();
    let mut spec = bench.spec();

    spec.program = PathBuf::from("caddy");
    assert_eq!(code(&spec, &bench), ErrorCode::InvalidPath);

    spec.program = bench.home.runtimes_dir().join("nothing").join("here");
    assert_eq!(code(&spec, &bench), ErrorCode::NotFound);
}

#[test]
fn service_ids_are_made_of_safe_characters() {
    let bench = Bench::new();
    for good in [
        "caddy",
        "mysql@8.4",
        "php-fpm@8.3.14",
        "php-fcgi@8.2.0",
        "a_b-c.d",
    ] {
        let mut spec = bench.spec();
        spec.id = good.to_owned();
        spec.validate(&bench.home)
            .unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    let too_long = "a".repeat(101);
    for bad in [
        "",
        "a b",
        "../x",
        "a/b",
        "a\\b",
        "a:b",
        "x..y",
        "naïve",
        too_long.as_str(),
    ] {
        let mut spec = bench.spec();
        spec.id = bad.to_owned();
        assert_eq!(code(&spec, &bench), ErrorCode::Internal, "{bad:?}");
    }
}

#[test]
fn environment_names_must_be_usable() {
    let bench = Bench::new();
    let mut spec = bench.spec();
    spec.env = vec![("PHPRC".into(), "D:\\lüna home\\config".into())];
    spec.validate(&bench.home).unwrap();

    for bad in [("", "x"), ("A=B", "x"), ("A\0B", "x"), ("OK", "va\0lue")] {
        spec.env = vec![(bad.0.into(), bad.1.into())];
        assert_eq!(code(&spec, &bench), ErrorCode::Internal, "{bad:?}");
    }
}

#[test]
fn the_working_directory_must_be_a_folder_inside_the_data_folder() {
    let bench = Bench::new();
    let mut spec = bench.spec();

    spec.cwd = Some(bench.home.run_dir());
    spec.validate(&bench.home).unwrap();

    spec.cwd = Some(bench.outside.clone());
    assert_eq!(code(&spec, &bench), ErrorCode::PathOutsideRoot);

    spec.cwd = Some(spec.program.clone());
    assert_eq!(code(&spec, &bench), ErrorCode::InvalidPath);

    spec.cwd = Some(PathBuf::from("relative"));
    assert_eq!(code(&spec, &bench), ErrorCode::InvalidPath);
}

#[test]
fn the_helper_programs_are_held_to_the_same_rule() {
    let bench = Bench::new();
    let inside = bench.program("mysqladmin");
    let outside = make_program(&bench.outside.join("mysqladmin"));
    let helper = |program: &Path| HelperCommand {
        program: program.to_path_buf(),
        args: vec!["ping".into()],
        env: Vec::new(),
    };

    let mut spec = bench.spec();
    spec.ready = ReadyCheck::Command(helper(&inside));
    spec.stop.method = StopMethod::Command(helper(&inside));
    spec.validate(&bench.home).unwrap();

    spec.ready = ReadyCheck::Command(helper(&outside));
    assert_eq!(code(&spec, &bench), ErrorCode::PathOutsideRoot);

    spec.ready = ReadyCheck::Immediately;
    spec.stop.method = StopMethod::Command(helper(&outside));
    assert_eq!(code(&spec, &bench), ErrorCode::PathOutsideRoot);

    let mut bad_env = helper(&inside);
    bad_env.env = vec![("A=B".into(), "x".into())];
    spec.stop.method = StopMethod::Command(bad_env);
    assert_eq!(code(&spec, &bench), ErrorCode::Internal);
}

#[test]
fn the_defaults_are_the_ones_in_the_service_spec_document() {
    let bench = Bench::new();
    let spec = bench.spec();

    assert_eq!(spec.ready, ReadyCheck::Tcp { port: 8101 });
    assert_eq!(spec.start_timeout, Duration::from_secs(30));
    assert_eq!(spec.stop.method, StopMethod::Terminate);
    assert_eq!(spec.stop.grace, Duration::from_secs(10));
    assert_eq!(spec.restart.max_restarts, 3);
}
