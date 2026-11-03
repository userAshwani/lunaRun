//! The six adapters. Each one knows the layout of its archives after extraction (with the registry's
//! `stripComponents` applied) and how to read the version it prints.
//!
//! Layouts were checked against the real archives: Windows PHP, Node, Caddy, MariaDB and Composer on this
//! Windows machine (`tests/runtimes_online.rs`, run with `--ignored`); the macOS and Linux static PHP, Caddy and
//! MySQL archives by listing their contents. What is still unverified is called out on each adapter.

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};

use super::adapter::{
    Binary, RuntimeAdapter, VersionCommand, VersionInfo, after, exe, leading_version,
};

fn binary(name: &'static str, path: PathBuf, required: bool) -> Binary {
    Binary {
        name,
        path,
        required,
    }
}

fn command(program: PathBuf, args: &[&'static str]) -> Option<VersionCommand> {
    Some(VersionCommand {
        program,
        args: args.to_vec(),
    })
}

/// PHP. Windows: one zip with `php.exe` and `php-cgi.exe` (FastCGI) at the top. macOS and Linux: two
/// static archives, `php` and `php-fpm`, extracted flat into the same folder.
pub struct Php;

impl RuntimeAdapter for Php {
    fn id(&self) -> &'static str {
        "php"
    }

    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary> {
        if os == Os::Windows {
            vec![
                binary("php", dir.join("php.exe"), true),
                binary("php-cgi", dir.join("php-cgi.exe"), true),
            ]
        } else {
            vec![
                binary("php", dir.join("php"), true),
                binary("php-fpm", dir.join("php-fpm"), true),
            ]
        }
    }

    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.to_path_buf()]
    }

    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand> {
        command(dir.join(exe(os, "php")), &["-v"])
    }

    /// `PHP 8.3.36 (cli) (built: ...)`
    fn parse_version(&self, output: &str) -> Option<String> {
        after(output, "PHP ").and_then(leading_version)
    }
}

/// Composer: one `composer.phar`. It cannot run without a PHP, so the install is checked by its content
/// instead: a PHP archive with the shebang line, and not empty. Running it is up to a project (M1.7).
pub struct Composer;

/// Composer's phar starts with this line.
const PHAR_SHEBANG: &[u8] = b"#!/usr/bin/env php";
/// The real `composer.phar` is about 3 MB. Anything this small is an error page or a truncated file.
const MIN_COMPOSER_BYTES: u64 = 500 * 1024;

impl RuntimeAdapter for Composer {
    fn id(&self) -> &'static str {
        "composer"
    }

    fn binaries(&self, dir: &Path, _os: Os) -> Vec<Binary> {
        vec![binary("composer", dir.join("composer.phar"), true)]
    }

    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.to_path_buf()]
    }

    fn version_command(&self, _dir: &Path, _os: Os) -> Option<VersionCommand> {
        None
    }

    fn parse_version(&self, _output: &str) -> Option<String> {
        None
    }

    fn verify(&self, dir: &Path, _os: Os, expected: &str) -> Result<VersionInfo> {
        let phar = dir.join("composer.phar");
        let broken = |why: String| {
            LunaError::new(
                ErrorCode::RuntimeBroken,
                format!("the installed composer does not work: {why}"),
            )
        };
        // Not `check_binaries`: a phar is run as `php composer.phar`, so it has no execute bit, and
        // requiring one would report every real Unix install as broken.
        if !phar.is_file() {
            return Err(broken(format!("{} is missing", phar.display())));
        }
        let size = fs::metadata(&phar)?.len();
        if size < MIN_COMPOSER_BYTES {
            return Err(broken(format!("composer.phar is only {size} bytes")));
        }
        let mut head = vec![0u8; PHAR_SHEBANG.len()];
        std::io::Read::read_exact(&mut fs::File::open(&phar)?, &mut head)?;
        if head != PHAR_SHEBANG {
            return Err(broken("composer.phar is not a PHP archive".to_owned()));
        }
        Ok(VersionInfo {
            version: expected.to_owned(),
            output: format!("composer.phar, {size} bytes (not run: it needs a PHP)"),
        })
    }
}

/// Node.js. Windows: `node.exe`, `npm.cmd`, `npx.cmd` at the top of the folder. macOS and Linux: `bin/node`,
/// `bin/npm`, `bin/npx` (the last two are symlinks into `lib/node_modules`).
pub struct Node;

impl RuntimeAdapter for Node {
    fn id(&self) -> &'static str {
        "node"
    }

    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary> {
        if os == Os::Windows {
            vec![
                binary("node", dir.join("node.exe"), true),
                binary("npm", dir.join("npm.cmd"), false),
                binary("npx", dir.join("npx.cmd"), false),
            ]
        } else {
            let bin = dir.join("bin");
            vec![
                binary("node", bin.join("node"), true),
                binary("npm", bin.join("npm"), false),
                binary("npx", bin.join("npx"), false),
            ]
        }
    }

    fn path_dirs(&self, dir: &Path, os: Os) -> Vec<PathBuf> {
        vec![if os == Os::Windows {
            dir.to_path_buf()
        } else {
            dir.join("bin")
        }]
    }

    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand> {
        let program = if os == Os::Windows {
            dir.join("node.exe")
        } else {
            dir.join("bin").join("node")
        };
        command(program, &["-v"])
    }

    /// `v24.22.2`
    fn parse_version(&self, output: &str) -> Option<String> {
        output.trim().strip_prefix('v').and_then(leading_version)
    }
}

/// MySQL. `bin/mysqld`, `bin/mysql`, `bin/mysqladmin` on every platform (`.exe` on Windows). The Windows,
/// macOS and Linux layouts are from the documented archive structure and a listing of the real
/// archives; only the Windows zip was extracted and run (`tests/runtimes_online.rs`).
pub struct Mysql;

impl RuntimeAdapter for Mysql {
    fn id(&self) -> &'static str {
        "mysql"
    }

    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary> {
        let bin = dir.join("bin");
        vec![
            binary("mysqld", bin.join(exe(os, "mysqld")), true),
            binary("mysql", bin.join(exe(os, "mysql")), false),
            binary("mysqladmin", bin.join(exe(os, "mysqladmin")), false),
        ]
    }

    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.join("bin")]
    }

    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand> {
        command(dir.join("bin").join(exe(os, "mysqld")), &["--version"])
    }

    /// `mysqld  Ver 8.4.13 for Linux on x86_64 (MySQL Community Server - GPL)`
    fn parse_version(&self, output: &str) -> Option<String> {
        after(output, " Ver ").and_then(leading_version)
    }
}

/// MariaDB. `bin/mariadbd` (the old name `mysqld` also exists on Windows), the client tools and
/// `mariadb-install-db`, which the first start needs (M1.4). Windows verified on the real zip. The Linux
/// archive was not extracted: its `bin/mariadbd` is expected from the documented layout.
pub struct Mariadb;

impl RuntimeAdapter for Mariadb {
    fn id(&self) -> &'static str {
        "mariadb"
    }

    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary> {
        let bin = dir.join("bin");
        vec![
            binary("mariadbd", bin.join(exe(os, "mariadbd")), true),
            binary("mariadb", bin.join(exe(os, "mariadb")), false),
            binary("mariadb-admin", bin.join(exe(os, "mariadb-admin")), false),
            binary(
                "mariadb-install-db",
                bin.join(exe(os, "mariadb-install-db")),
                false,
            ),
        ]
    }

    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.join("bin")]
    }

    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand> {
        command(dir.join("bin").join(exe(os, "mariadbd")), &["--version"])
    }

    /// `mariadbd  Ver 11.8.9-MariaDB for Win64 on AMD64 (mariadb.org binary distribution)`
    fn parse_version(&self, output: &str) -> Option<String> {
        after(output, " Ver ").and_then(leading_version)
    }
}

/// Caddy: one `caddy` (`caddy.exe`) at the top of the folder, next to `LICENSE` and `README.md`.
pub struct Caddy;

impl RuntimeAdapter for Caddy {
    fn id(&self) -> &'static str {
        "caddy"
    }

    fn binaries(&self, dir: &Path, os: Os) -> Vec<Binary> {
        vec![binary("caddy", dir.join(exe(os, "caddy")), true)]
    }

    fn path_dirs(&self, dir: &Path, _os: Os) -> Vec<PathBuf> {
        vec![dir.to_path_buf()]
    }

    fn version_command(&self, dir: &Path, os: Os) -> Option<VersionCommand> {
        command(dir.join(exe(os, "caddy")), &["version"])
    }

    /// `v2.11.7 h1:...`
    fn parse_version(&self, output: &str) -> Option<String> {
        output.trim().strip_prefix('v').and_then(leading_version)
    }
}

/// All adapters, in registry id order.
pub fn adapters() -> [&'static dyn RuntimeAdapter; 6] {
    [&Caddy, &Composer, &Mariadb, &Mysql, &Node, &Php]
}

/// The adapter for a registry runtime id.
pub fn adapter_for(id: &str) -> Option<&'static dyn RuntimeAdapter> {
    adapters().into_iter().find(|adapter| adapter.id() == id)
}
