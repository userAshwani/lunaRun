//! The PHP and Caddy services: the specs and generated config built from installed runtimes, and Caddy's validate and
//! reload (FR-SV-02, FR-ST-03, D-028). The runtimes here are folders with stand-in program files; the real PHP and
//! Caddy are run by the ignored test in `web_online.rs`. Every home has a space and a non-ASCII letter in its path
//! (NFR-07).

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::path::Path;
use std::time::Duration;

use luna_core::diagnostics::{Arch, Os};
use luna_core::runtimes::{InstallReceipt, InstalledRuntime, php_config_paths};
use luna_core::services::{
    CaddyDefinition, PhpDefinition, PortAllocator, PortRanges, ReadyCheck, Site, StopMethod,
    caddy_definition, php_definition,
};
use luna_core::{ErrorCode, LunaHome};
use stub_service::*;
use tokio_util::sync::CancellationToken;

const PLATFORMS: [Os; 3] = [Os::Windows, Os::Linux, Os::Macos];
const LIMIT: Duration = Duration::from_secs(30);

/// Ranges of ports of our own, so the tests do not depend on what runs on this machine.
fn own_ranges() -> PortRanges {
    PortRanges {
        php_fastcgi: free_block(16),
        caddy_admin: free_block(1),
        ..PortRanges::default()
    }
}

fn allocator(home: &LunaHome) -> PortAllocator {
    PortAllocator::new(home.clone()).with_ranges(own_ranges())
}

fn program(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "stand-in").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Forward slashes, as the generated files and arguments write a path.
fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn receipt(runtime: &str, version: &str, os: Os) -> InstallReceipt {
    InstallReceipt::new(runtime, version, os, Arch::X86_64, false, &[])
}

/// An installed PHP: the program files of the layout for `os` and the `php.ini` that the install generated.
fn installed_php(home: &LunaHome, version: &str, os: Os) -> InstalledRuntime {
    let dir = home.runtime_dir("php", version).unwrap();
    let names: &[&str] = if os == Os::Windows {
        &["php.exe", "php-cgi.exe"]
    } else {
        &["php", "php-fpm"]
    };
    for name in names {
        program(&dir.join(name));
    }
    let paths = php_config_paths(home, version).unwrap();
    fs::create_dir_all(&paths.dir).unwrap();
    fs::write(&paths.ini, "; generated at install\n").unwrap();
    InstalledRuntime {
        runtime: "php".to_owned(),
        version: version.to_owned(),
        dir,
        receipt: receipt("php", version, os),
    }
}

fn installed_caddy(home: &LunaHome, os: Os) -> InstalledRuntime {
    let dir = home.runtime_dir("caddy", "2.11.7").unwrap();
    program(&dir.join(if os == Os::Windows {
        "caddy.exe"
    } else {
        "caddy"
    }));
    InstalledRuntime {
        runtime: "caddy".to_owned(),
        version: "2.11.7".to_owned(),
        dir,
        receipt: receipt("caddy", "2.11.7", os),
    }
}

fn php(home: &LunaHome, ports: &PortAllocator, version: &str, os: Os, n: u16) -> PhpDefinition {
    php_definition(home, ports, &installed_php(home, version, os), os, n).unwrap()
}

fn site(name: &str, port: u16, root: &Path, php_ports: &[u16]) -> Site {
    Site {
        name: name.to_owned(),
        port,
        root: root.to_path_buf(),
        php_ports: php_ports.to_vec(),
    }
}

fn caddy(bench: &Shared, ports: &PortAllocator, sites: &[Site], os: Os) -> CaddyDefinition {
    caddy_definition(
        &bench.home,
        ports,
        &installed_caddy(&bench.home, os),
        sites,
        os,
    )
    .unwrap()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

// ---------------------------------------------------------------------------------------------
// PHP on Windows: a pool of php-cgi processes
// ---------------------------------------------------------------------------------------------

#[test]
fn a_windows_pool_is_one_service_per_php_cgi_on_consecutive_ports() {
    let bench = private("web-php-windows");
    let ports = allocator(&bench.home);

    let d = php(&bench.home, &ports, "8.3.36", Os::Windows, 3);

    assert_eq!(d.version, "8.3.36");
    assert_eq!(d.pool_id, "php-fcgi@8.3.36");
    assert!(d.config_file.is_none());
    assert_eq!(d.ports.len(), 3);
    assert_eq!(d.ports[1], d.ports[0] + 1);
    assert_eq!(d.ports[2], d.ports[0] + 2);
    assert_eq!(d.specs.len(), 3);

    let runtime = bench.home.runtime_dir("php", "8.3.36").unwrap();
    let paths = php_config_paths(&bench.home, "8.3.36").unwrap();
    for (index, spec) in d.specs.iter().enumerate() {
        let port = d.ports[index];
        assert_eq!(spec.id, format!("php-fcgi@8.3.36-w{}", index + 1));
        assert_eq!(spec.program, runtime.join("php-cgi.exe"));
        assert_eq!(
            spec.args,
            [
                "-b".to_owned(),
                format!("127.0.0.1:{port}"),
                "-c".to_owned(),
                paths.ini.display().to_string()
            ]
        );
        assert_eq!(spec.ready, ReadyCheck::Tcp { port });
        assert_eq!(spec.ports, [port]);
        assert_eq!(spec.cwd.as_deref(), Some(runtime.as_path()));
        assert_eq!(spec.stop.method, StopMethod::Kill, "it is stateless");
        spec.validate(&bench.home).unwrap();
    }
}

#[test]
fn every_php_cgi_reads_the_generated_config_and_is_recycled_not_counted_as_crashed() {
    let bench = private("web-php-env");
    let ports = allocator(&bench.home);

    let d = php(&bench.home, &ports, "8.3.36", Os::Windows, 2);

    let paths = php_config_paths(&bench.home, "8.3.36").unwrap();
    for spec in &d.specs {
        let env = |name: &str| {
            spec.env
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(env("PHPRC"), Some(paths.dir.display().to_string()));
        assert_eq!(
            env("PHP_INI_SCAN_DIR"),
            Some(paths.conf_d.display().to_string())
        );
        assert_eq!(env("PHP_FCGI_MAX_REQUESTS").as_deref(), Some("500"));
        assert!(
            spec.restart.recycle_on_exit,
            "php-cgi exits by design after its request limit"
        );
    }
}

#[test]
fn a_windows_pool_of_one_to_sixteen_is_allowed_and_nothing_else() {
    let bench = private("web-php-size");
    let ports = allocator(&bench.home);
    let runtime = installed_php(&bench.home, "8.3.36", Os::Windows);

    for bad in [0, 17, 1000] {
        let error = php_definition(&bench.home, &ports, &runtime, Os::Windows, bad).unwrap_err();
        assert_eq!(error.code, ErrorCode::Internal, "{bad} workers");
    }
    for good in [1, 16] {
        let d = php_definition(&bench.home, &ports, &runtime, Os::Windows, good).unwrap();
        assert_eq!(d.specs.len(), usize::from(good));
    }
}

#[test]
fn a_pool_keeps_its_ports_and_gets_new_ones_when_its_size_changes() {
    let bench = private("web-php-stable");
    let ports = allocator(&bench.home);

    let first = php(&bench.home, &ports, "8.3.36", Os::Windows, 2);
    let again = php(&bench.home, &ports, "8.3.36", Os::Windows, 2);
    assert_eq!(first.ports, again.ports, "the same ports as before");

    let bigger = php(&bench.home, &ports, "8.3.36", Os::Windows, 4);
    assert_eq!(bigger.ports.len(), 4);
    assert_eq!(bigger.specs.len(), 4);
}

#[test]
fn two_php_versions_run_side_by_side_on_ports_of_their_own() {
    let bench = private("web-php-versions");
    let ports = allocator(&bench.home);

    for os in PLATFORMS {
        let a = php(&bench.home, &ports, "8.3.36", os, 2);
        let b = php(&bench.home, &ports, "8.4.12", os, 2);

        assert_ne!(a.pool_id, b.pool_id);
        for port in &a.ports {
            assert!(!b.ports.contains(port), "{os:?}: no port twice");
        }
        let ids: Vec<&str> = a
            .specs
            .iter()
            .chain(&b.specs)
            .map(|s| s.id.as_str())
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "{os:?}: no service id twice");
    }
}

// ---------------------------------------------------------------------------------------------
// PHP on macOS and Linux: php-fpm
// ---------------------------------------------------------------------------------------------

#[test]
fn on_macos_and_linux_php_is_one_php_fpm_in_the_foreground_with_a_generated_config() {
    for os in [Os::Linux, Os::Macos] {
        let bench = private(&format!("web-php-fpm-{os:?}"));
        let ports = allocator(&bench.home);

        // The size of a Windows pool means nothing here.
        let d = php(&bench.home, &ports, "8.3.36", os, 0);

        assert_eq!(d.pool_id, "php-fpm@8.3.36");
        assert_eq!(d.ports.len(), 1);
        assert_eq!(d.specs.len(), 1);
        let spec = &d.specs[0];
        let port = d.ports[0];
        let paths = php_config_paths(&bench.home, "8.3.36").unwrap();
        let conf = d.config_file.clone().unwrap();
        assert_eq!(conf, paths.dir.join("php-fpm.conf"));

        assert_eq!(spec.id, "php-fpm@8.3.36");
        assert_eq!(
            spec.program,
            bench
                .home
                .runtime_dir("php", "8.3.36")
                .unwrap()
                .join("php-fpm")
        );
        assert_eq!(
            spec.args,
            [
                "--nodaemonize".to_owned(),
                "--fpm-config".to_owned(),
                conf.display().to_string(),
                "-c".to_owned(),
                paths.ini.display().to_string()
            ]
        );
        assert_eq!(spec.ready, ReadyCheck::Tcp { port });
        assert_eq!(spec.ports, [port]);
        assert_eq!(spec.stop.method, StopMethod::Quit);
        assert_eq!(spec.stop.grace, Duration::from_secs(10));
        assert!(!spec.restart.recycle_on_exit);
        assert!(spec.env.iter().any(|(key, _)| key == "PHPRC"));
        assert!(spec.env.iter().any(|(key, _)| key == "PHP_INI_SCAN_DIR"));
        assert!(
            spec.env
                .iter()
                .all(|(key, _)| key != "PHP_FCGI_MAX_REQUESTS"),
            "that is php-cgi's"
        );
        spec.validate(&bench.home).unwrap();

        // One log tab: php-fpm's own log, which lunaRun reads.
        let log = bench.home.logs_dir().join("php-fpm-8.3.36.log");
        assert_eq!(spec.log_files.len(), 1);
        assert_eq!(spec.log_files[0].source, "PHP-FPM log");
        assert_eq!(spec.log_files[0].path, log);

        let text = read(&conf);
        for line in [
            "daemonize = no".to_owned(),
            format!("error_log = \"{}\"", fwd(&log)),
            format!("listen = 127.0.0.1:{port}"),
            "pm = dynamic".to_owned(),
            "pm.max_children = 8".to_owned(),
            "catch_workers_output = yes".to_owned(),
            "clear_env = no".to_owned(),
        ] {
            assert!(
                text.lines().any(|l| l == line),
                "{os:?}: {line:?} missing from\n{text}"
            );
        }
        assert!(!text.contains("{{"), "no placeholder is left");
    }
}

#[test]
fn the_php_fpm_config_is_written_again_every_time() {
    let bench = private("web-php-fpm-rewrite");
    let ports = allocator(&bench.home);
    let d = php(&bench.home, &ports, "8.3.36", Os::Linux, 1);
    let conf = d.config_file.unwrap();
    fs::write(&conf, "; somebody edited this\n").unwrap();

    let again = php(&bench.home, &ports, "8.3.36", Os::Linux, 1);

    assert_eq!(again.config_file.as_deref(), Some(conf.as_path()));
    let text = read(&conf);
    assert!(text.contains("listen = 127.0.0.1:"), "{text}");
    assert!(!text.contains("somebody edited"), "and loses hand edits");
}

#[test]
fn windows_writes_no_php_fpm_config() {
    let bench = private("web-php-no-fpm-conf");
    let ports = allocator(&bench.home);
    php(&bench.home, &ports, "8.3.36", Os::Windows, 2);

    let paths = php_config_paths(&bench.home, "8.3.36").unwrap();
    assert!(!paths.dir.join("php-fpm.conf").exists());
}

#[test]
fn a_php_whose_generated_ini_is_gone_is_broken_and_something_else_is_not_php() {
    let bench = private("web-php-broken");
    let ports = allocator(&bench.home);

    let runtime = installed_php(&bench.home, "8.3.36", Os::Windows);
    fs::remove_file(php_config_paths(&bench.home, "8.3.36").unwrap().ini).unwrap();
    let error = php_definition(&bench.home, &ports, &runtime, Os::Windows, 2).unwrap_err();
    assert_eq!(error.code, ErrorCode::RuntimeBroken);
    assert!(error.message.contains("php.ini"), "{}", error.message);

    let other = installed_caddy(&bench.home, Os::Windows);
    let error = php_definition(&bench.home, &ports, &other, Os::Windows, 2).unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
}

// ---------------------------------------------------------------------------------------------
// Caddy: the Caddyfile and the spec
// ---------------------------------------------------------------------------------------------

fn lines(parts: &[&str]) -> String {
    let mut text = parts.join("\n");
    text.push('\n');
    text
}

#[test]
fn the_caddyfile_has_a_global_block_and_one_block_per_site() {
    let bench = private("web-caddy-file");
    let ports = allocator(&bench.home);
    let shop = bench.scratch("Projekte größe").join("shop").join("public");
    let docs = bench.scratch("Projekte größe").join("docs");
    let sites = [
        site("shop", 8101, &shop, &[9101, 9102]),
        site("docs", 8102, &docs, &[]),
    ];

    let d = caddy(&bench, &ports, &sites, Os::current());

    let logs = fwd(&bench.home.logs_dir());
    let expected = lines(&[
        "# Generated by lunaRun. Changes made here are overwritten whenever the sites change.",
        "{",
        &format!("\tadmin 127.0.0.1:{}", d.admin_port),
        "\tauto_https off",
        "\tpersist_config off",
        "}",
        "",
        "http://localhost:8101, http://127.0.0.1:8101 {",
        "\tbind 127.0.0.1",
        &format!("\troot * \"{}\"", fwd(&shop)),
        "\tencode gzip",
        "\tphp_fastcgi 127.0.0.1:9101 127.0.0.1:9102",
        "\tfile_server",
        "\tlog {",
        &format!("\t\toutput file \"{logs}/sites/shop.access.log\""),
        "\t}",
        "}",
        "",
        "http://localhost:8102, http://127.0.0.1:8102 {",
        "\tbind 127.0.0.1",
        &format!("\troot * \"{}\"", fwd(&docs)),
        "\tencode gzip",
        "\tfile_server",
        "\tlog {",
        &format!("\t\toutput file \"{logs}/sites/docs.access.log\""),
        "\t}",
        "}",
    ]);
    assert_eq!(read(&d.config_file), expected);
    assert_eq!(
        d.config_file,
        bench.home.config_dir().join("caddy").join("Caddyfile")
    );
}

#[test]
fn caddy_listens_on_the_admin_port_and_every_site_and_is_ready_when_the_first_site_answers() {
    for os in PLATFORMS {
        let bench = private(&format!("web-caddy-spec-{os:?}"));
        let ports = allocator(&bench.home);
        let root = bench.scratch("proj");
        let sites = [
            site("shop", 8101, &root, &[9101]),
            site("docs", 8102, &root, &[]),
        ];

        let d = caddy(&bench, &ports, &sites, os);

        let spec = &d.spec;
        let program =
            bench
                .home
                .runtime_dir("caddy", "2.11.7")
                .unwrap()
                .join(if os == Os::Windows {
                    "caddy.exe"
                } else {
                    "caddy"
                });
        assert_eq!(spec.id, "caddy");
        assert_eq!(spec.program, program);
        assert_eq!(
            spec.args,
            [
                "run".to_owned(),
                "--config".to_owned(),
                fwd(&d.config_file),
                "--adapter".to_owned(),
                "caddyfile".to_owned()
            ]
        );
        assert_eq!(spec.ports, [d.admin_port, 8101, 8102], "{os:?}");
        assert_eq!(spec.ready, ReadyCheck::Tcp { port: 8101 });
        assert_eq!(spec.cwd, d.config_file.parent().map(Path::to_path_buf));
        spec.validate(&bench.home).unwrap();

        // Caddy keeps its own state inside LUNA_HOME, not in the user's profile.
        let env = |name: &str| {
            spec.env
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            env("XDG_DATA_HOME"),
            Some(fwd(&bench.home.data_dir().join("caddy")))
        );
        assert_eq!(
            env("XDG_CONFIG_HOME"),
            Some(fwd(&bench.home.config_dir().join("caddy").join("state")))
        );

        // The two tabs, one per site.
        let tabs: Vec<&str> = spec.log_files.iter().map(|l| l.source.as_str()).collect();
        assert_eq!(tabs, ["shop access log", "docs access log"]);
        assert_eq!(
            spec.log_files[0].path,
            bench.home.logs_dir().join("sites").join("shop.access.log")
        );
    }
}

#[test]
fn caddy_stops_through_its_admin_api_and_is_checked_and_reloaded_through_the_caddyfile() {
    let bench = private("web-caddy-commands");
    let ports = allocator(&bench.home);
    let d = caddy(&bench, &ports, &[], Os::current());
    let admin = format!("127.0.0.1:{}", d.admin_port);
    let file = fwd(&d.config_file);

    let StopMethod::Command(stop) = &d.spec.stop.method else {
        panic!("Caddy is stopped through its admin API");
    };
    assert_eq!(stop.program, d.spec.program);
    assert_eq!(stop.args, ["stop", "--address", admin.as_str()]);
    assert_eq!(d.spec.stop.grace, Duration::from_secs(10));
    assert_eq!(
        d.validate_command.args,
        [
            "validate",
            "--config",
            file.as_str(),
            "--adapter",
            "caddyfile"
        ]
    );
    assert_eq!(
        d.reload_command.args,
        [
            "reload",
            "--config",
            file.as_str(),
            "--adapter",
            "caddyfile",
            "--address",
            admin.as_str()
        ]
    );
    // The helpers keep Caddy's state inside LUNA_HOME too.
    for helper in [stop, &d.validate_command, &d.reload_command] {
        assert!(helper.env.iter().any(|(key, _)| key == "XDG_DATA_HOME"));
    }
    d.spec.validate(&bench.home).unwrap();
}

#[test]
fn with_no_sites_caddy_has_only_its_admin_api_and_is_ready_when_that_answers() {
    let bench = private("web-caddy-empty");
    let ports = allocator(&bench.home);

    let d = caddy(&bench, &ports, &[], Os::current());

    let text = read(&d.config_file);
    assert!(text.contains(&format!("admin 127.0.0.1:{}", d.admin_port)));
    assert!(!text.contains("http://"), "{text}");
    assert_eq!(d.spec.ports, [d.admin_port]);
    assert_eq!(d.spec.ready, ReadyCheck::Tcp { port: d.admin_port });
    assert!(d.spec.log_files.is_empty());
}

#[test]
fn the_admin_port_is_kept_and_the_caddyfile_follows_the_sites() {
    let bench = private("web-caddy-rewrite");
    let ports = allocator(&bench.home);
    let root = bench.scratch("proj");

    let first = caddy(
        &bench,
        &ports,
        &[site("shop", 8101, &root, &[])],
        Os::current(),
    );
    fs::write(&first.config_file, "# somebody edited this\n").unwrap();
    let second = caddy(
        &bench,
        &ports,
        &[site("blog", 8103, &root, &[])],
        Os::current(),
    );

    assert_eq!(second.admin_port, first.admin_port, "the same admin port");
    let text = read(&second.config_file);
    assert!(text.contains("localhost:8103"), "{text}");
    assert!(!text.contains("8101"), "the removed site is gone");
    assert!(!text.contains("somebody edited"), "and so are hand edits");
}

#[test]
fn sites_that_clash_or_cannot_be_written_are_refused() {
    let bench = private("web-caddy-refuse");
    let ports = allocator(&bench.home);
    let runtime = installed_caddy(&bench.home, Os::current());
    let root = bench.scratch("proj");
    let admin = ports
        .assign("caddy", luna_core::services::PortPurpose::CaddyAdmin)
        .unwrap()
        .port;
    let attempt = |sites: &[Site]| {
        caddy_definition(&bench.home, &ports, &runtime, sites, Os::current()).unwrap_err()
    };

    for (what, sites) in [
        (
            "a name twice",
            vec![site("a", 8101, &root, &[]), site("a", 8102, &root, &[])],
        ),
        (
            "a port twice",
            vec![site("a", 8101, &root, &[]), site("b", 8101, &root, &[])],
        ),
        ("the admin port", vec![site("a", admin, &root, &[])]),
        ("port 0", vec![site("a", 0, &root, &[])]),
        ("PHP port 0", vec![site("a", 8101, &root, &[0])]),
        ("a name with a slash", vec![site("a/b", 8101, &root, &[])]),
        ("a name that climbs", vec![site("..", 8101, &root, &[])]),
        ("an empty name", vec![site("", 8101, &root, &[])]),
        (
            "a name that is too long",
            vec![site(&"x".repeat(41), 8101, &root, &[])],
        ),
    ] {
        let code = attempt(&sites).code;
        assert!(
            matches!(code, ErrorCode::Internal | ErrorCode::InvalidPath),
            "{what}: {code:?}"
        );
    }

    // A folder that cannot be written into a Caddyfile faithfully: a quote, a line break, braces (a placeholder).
    for bad in ["a\"b", "a\nb", "a{b}", "{env.HOME}"] {
        let error = attempt(&[site(
            "a",
            8101,
            Path::new(&format!("/work/{bad}/public")),
            &[],
        )]);
        assert_eq!(error.code, ErrorCode::InvalidPath, "{bad:?}");
    }

    // Nothing of the refused attempts was written.
    let wrong = installed_php(&bench.home, "8.3.36", Os::current());
    let error = caddy_definition(&bench.home, &ports, &wrong, &[], Os::current()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
}

#[test]
fn a_site_folder_is_never_touched() {
    let bench = private("web-caddy-readonly");
    let ports = allocator(&bench.home);
    let root = bench.scratch("proj").join("untouched");

    caddy(
        &bench,
        &ports,
        &[site("a", 8101, &root, &[])],
        Os::current(),
    );

    assert!(
        !root.exists(),
        "the project folder was not created or written"
    );
}

// ---------------------------------------------------------------------------------------------
// Caddy: validate and reload
// ---------------------------------------------------------------------------------------------

fn data_arg(bench: &Shared) -> String {
    bench.scratch("t").join("data").display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caddyfile_that_validates_passes() {
    let bench = private("web-caddy-validate-ok");
    let ports = allocator(&bench.home);
    let mut d = caddy(&bench, &ports, &[], Os::current());
    let data = data_arg(&bench);
    d.validate_command = bench.helper("init-ok", &[("LUNA_STUB_DATADIR", &data)]);

    d.validate(LIMIT, &CancellationToken::new()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caddyfile_that_is_rejected_says_why_and_keeps_what_caddy_said() {
    let bench = private("web-caddy-validate-fail");
    let ports = allocator(&bench.home);
    let mut d = caddy(&bench, &ports, &[], Os::current());
    let data = data_arg(&bench);
    d.validate_command = bench.helper("init-fail", &[("LUNA_STUB_DATADIR", &data)]);

    let error = d
        .validate(LIMIT, &CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ConfigInvalid);
    assert!(error.message.contains("exit code 3"), "{}", error.message);
    let details = error.details.as_ref().unwrap();
    assert!(
        details["tail"].to_string().contains("boom"),
        "what Caddy said is in the error: {details}"
    );
    assert_eq!(details["log"], d.check_log.display().to_string());
    assert!(read(&d.check_log).contains("boom"), "and kept in the log");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reload_that_fails_is_a_reload_error_and_one_that_works_is_not() {
    let bench = private("web-caddy-reload");
    let ports = allocator(&bench.home);
    let mut d = caddy(&bench, &ports, &[], Os::current());
    let data = data_arg(&bench);

    d.reload_command = bench.helper("init-ok", &[("LUNA_STUB_DATADIR", &data)]);
    d.reload(LIMIT, &CancellationToken::new()).await.unwrap();

    d.reload_command = bench.helper("init-fail", &[("LUNA_STUB_DATADIR", &data)]);
    let error = d
        .reload(LIMIT, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ReloadFailed);
    assert!(
        error.details.as_ref().unwrap()["tail"]
            .to_string()
            .contains("boom")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_check_that_hangs_is_given_up_on_and_one_that_is_cancelled_is_stopped() {
    let bench = private("web-caddy-hang");
    let ports = allocator(&bench.home);
    let mut d = caddy(&bench, &ports, &[], Os::current());
    let heartbeat = bench.scratch("t").join("heartbeat");
    let hang = |bench: &Shared| {
        bench.helper(
            "serve",
            &[
                ("LUNA_STUB_NO_LISTEN", "1"),
                ("LUNA_STUB_HEARTBEAT", heartbeat.to_str().unwrap()),
            ],
        )
    };

    d.validate_command = hang(&bench);
    let begun = std::time::Instant::now();
    let error = d
        .validate(Duration::from_secs(1), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConfigInvalid);
    assert!(
        error.message.contains("did not answer within"),
        "{}",
        error.message
    );
    assert!(begun.elapsed() < Duration::from_secs(10));
    assert_process_gone(&heartbeat).await;

    d.reload_command = hang(&bench);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        trigger.cancel();
    });
    let error = d.reload(LIMIT, &cancel).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_process_gone(&heartbeat).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caddy_that_is_missing_is_a_failure_not_a_panic() {
    let bench = private("web-caddy-missing");
    let ports = allocator(&bench.home);
    let mut d = caddy(&bench, &ports, &[], Os::current());
    d.validate_command.program = bench.home.runtimes_dir().join("not-there.exe");

    let error = d
        .validate(LIMIT, &CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ConfigInvalid);
    assert!(
        error.message.contains("could not be started"),
        "{}",
        error.message
    );
}
