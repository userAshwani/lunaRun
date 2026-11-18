//! A terminal in a project's folder with **its own** PHP, Composer and Node first on `PATH` (FR-PJ-08; docs/SERVICES.md
//! section 5; D-040).
//!
//! The environment is for **that terminal only**: nothing global or per user is changed (shims on the user's `PATH` are
//! Phase 2, opt-in). [`plan`] is a pure function of the installed runtimes and of what it is told about the system
//! (`OpenEnv`), so every system is tested on any machine; [`open`] reads what a project resolves to, writes the files
//! the plan names (all **inside LUNA_HOME**) and starts the terminal with an argument array and no shell string
//! (golden rule 6).
//!
//! - `PATH`: the folder of the project's PHP, the folder of a generated `composer` shim, the folders of its Node (when one
//!   is installed), then the `PATH` the terminal would have had. `PHPRC` and `PHP_INI_SCAN_DIR` point PHP at the
//!   `php.ini` lunaRun generated ([`php_environment`]); `LUNA_PROJECT` is the project's id.
//! - The `composer` shim runs `php composer.phar "$@"` with **that** PHP: `composer.bat` on Windows, a shell script elsewhere.
//! - Windows: PowerShell in a console window of its own, in the folder, with the environment given to the process (the
//!   system shows it in the user's default terminal). macOS and Linux: a script that exports the same things, goes to the
//!   folder and starts the user's shell, opened by `open -a Terminal` or by the first terminal program found.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};
use crate::opener::{OpenCommand, OpenEnv, spawn};
use crate::paths::LunaHome;
use crate::projects::{Project, ProjectStore};
use crate::registry::RuntimeVersion;
use crate::runtimes::{InstalledRuntime, adapter_for, list_installed, php_environment};
use crate::sites::SiteManager;
use crate::state::write_atomic;

/// The installed runtimes a project's terminal gets.
pub struct Toolchain<'a> {
    pub php: &'a InstalledRuntime,
    /// The Node the project resolves to, when one is installed.
    pub node: Option<&'a InstalledRuntime>,
    /// The newest installed Composer, when there is one.
    pub composer: Option<&'a InstalledRuntime>,
}

/// A file the plan needs on disk before the terminal starts. Always inside LUNA_HOME.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub content: String,
    /// A script that must be allowed to run (macOS and Linux).
    pub executable: bool,
}

/// Everything needed to open the terminal: the files to write, and what to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPlan {
    pub files: Vec<GeneratedFile>,
    pub command: OpenCommand,
}

/// Linux terminal programs, in the order they are tried, with the argument that makes them run a program.
const LINUX_TERMINALS: [(&str, &str); 4] = [
    ("x-terminal-emulator", "-e"),
    ("gnome-terminal", "--"),
    ("konsole", "-e"),
    ("xterm", "-e"),
];

fn path_separator(os: Os) -> &'static str {
    if os == Os::Windows { ";" } else { ":" }
}

/// Characters a version or an id may have to be written into a script or a command line.
fn is_plain(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn plain(what: &str, text: &str) -> Result<()> {
    if is_plain(text) {
        return Ok(());
    }
    Err(LunaError::new(
        ErrorCode::InvalidInput,
        format!("{text:?} is not a plain {what}"),
    )
    .with_details(json!({ "field": what })))
}

/// A path as text for a script. The terminal is started from files lunaRun writes, so a path that is not text cannot be used.
fn text_of(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        LunaError::new(
            ErrorCode::InvalidPath,
            format!("{} cannot be written into a script", path.display()),
        )
        .with_details(json!({ "path": path.display().to_string() }))
    })
}

/// `'it''s'` for a Unix shell: single quotes, a quote inside closes, escapes and reopens.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The program of `runtime` called `name`, from its adapter.
fn program_of(runtime: &InstalledRuntime, name: &str, os: Os) -> Result<PathBuf> {
    adapter_for(&runtime.runtime)
        .and_then(|adapter| {
            adapter
                .binaries(&runtime.dir, os)
                .into_iter()
                .find(|binary| binary.name == name)
                .map(|binary| binary.path)
        })
        .ok_or_else(|| LunaError::internal(format!("{} has no program {name}", runtime.runtime)))
}

fn dirs_of(runtime: &InstalledRuntime, os: Os) -> Vec<PathBuf> {
    adapter_for(&runtime.runtime)
        .map(|adapter| adapter.path_dirs(&runtime.dir, os))
        .unwrap_or_default()
}

/// The folder of the `composer` shim of one PHP: `config/shims/php-8.3.14`.
pub fn shim_dir(home: &LunaHome, php_version: &str) -> PathBuf {
    home.config_dir()
        .join("shims")
        .join(format!("php-{php_version}"))
}

/// A path inside LUNA_HOME as a batch-file path **relative to the shim's own folder** (`%~dp0..\..\..\runtimes\...`).
/// A batch file is read in the console's code page, not UTF-8, so a folder with a letter such as `ü` or `é` (a Windows
/// profile called José) written into it would not be found; relative to the shim there is no such letter in the file.
/// `None` for a path outside LUNA_HOME.
fn relative_to_shim(home: &LunaHome, shim_dir: &Path, target: &Path) -> Option<String> {
    let up = shim_dir
        .strip_prefix(home.root())
        .ok()?
        .components()
        .count();
    let below = target.strip_prefix(home.root()).ok()?;
    let mut text = String::from("%~dp0");
    for _ in 0..up {
        text.push_str("..\\");
    }
    let parts: Vec<String> = below
        .components()
        .map(|c| c.as_os_str().to_str().map(|s| s.replace('%', "%%")))
        .collect::<Option<_>>()?;
    text.push_str(&parts.join("\\"));
    Some(text)
}

/// The `composer` shim: runs `php composer.phar` with the project's PHP. A path is quoted for its own shell, and `%`
/// is doubled in a batch file.
fn composer_shim(
    os: Os,
    home: &LunaHome,
    php_version: &str,
    php: &Path,
    phar: &Path,
) -> Result<GeneratedFile> {
    let note = format!("Generated by lunaRun: runs Composer with PHP {php_version}.");
    if os == Os::Windows {
        let dir = shim_dir(home, php_version);
        let name = |path: &Path| -> Result<String> {
            match relative_to_shim(home, &dir, path) {
                Some(relative) => Ok(relative),
                None => Ok(text_of(path)?.replace('%', "%%")),
            }
        };
        let content = format!(
            "@echo off\r\nrem {note}\r\n\"{}\" \"{}\" %*\r\n",
            name(php)?,
            name(phar)?
        );
        return Ok(GeneratedFile {
            path: dir.join("composer.bat"),
            content,
            executable: false,
        });
    }
    let (php, phar) = (text_of(php)?, text_of(phar)?);
    Ok(GeneratedFile {
        path: shim_dir(home, php_version).join("composer"),
        content: format!(
            "#!/bin/sh\n# {note}\nexec {} {} \"$@\"\n",
            sh_quote(&php),
            sh_quote(&phar)
        ),
        executable: true,
    })
}

/// The one line the terminal says first, so it is clear whose environment it is.
fn banner(tools: &Toolchain<'_>) -> String {
    let mut text = format!("lunaRun: PHP {}", tools.php.version);
    if let Some(node) = tools.node {
        text.push_str(&format!(", Node.js {}", node.version));
    }
    if let Some(composer) = tools.composer {
        text.push_str(&format!(", Composer {}", composer.version));
    }
    text
}

/// Build the plan for a terminal in `folder` for project `project_id`.
///
/// `original_path` is the `PATH` the terminal would otherwise have (Windows and the environment given to a process; macOS
/// and Linux scripts use the terminal's own at the time it starts).
///
/// # Errors
/// `INVALID_INPUT` for an id or a version that is not plain text, `INVALID_PATH` for a path that cannot be written into
/// a script or that contains the `PATH` separator, `NOT_FOUND` for a folder that is not there or (with `details.target`
/// `terminal`) when no terminal program is found.
pub fn plan(
    home: &LunaHome,
    project_id: &str,
    folder: &Path,
    tools: &Toolchain<'_>,
    original_path: Option<&OsStr>,
    env: &OpenEnv<'_>,
) -> Result<TerminalPlan> {
    let os = env.os;
    if os == Os::Unknown {
        return Err(LunaError::internal(
            "a terminal is not supported on this system",
        ));
    }
    plain("project id", project_id)?;
    plain("PHP version", &tools.php.version)?;
    if let Some(node) = tools.node {
        plain("Node.js version", &node.version)?;
    }
    if let Some(composer) = tools.composer {
        plain("Composer version", &composer.version)?;
    }
    if !(env.exists)(folder) {
        return Err(LunaError::new(
            ErrorCode::NotFound,
            format!("the project folder {} is not there", folder.display()),
        )
        .with_details(json!({ "path": folder.display().to_string() })));
    }

    let mut files = Vec::new();
    let mut dirs = dirs_of(tools.php, os);
    if let Some(composer) = tools.composer {
        let shim = composer_shim(
            os,
            home,
            &tools.php.version,
            &program_of(tools.php, "php", os)?,
            &program_of(composer, "composer", os)?,
        )?;
        dirs.push(shim_dir(home, &tools.php.version));
        files.push(shim);
    }
    if let Some(node) = tools.node {
        dirs.extend(dirs_of(node, os));
    }

    // The environment: PATH first, then what PHP needs to find its generated configuration.
    let separator = path_separator(os);
    let mut dir_texts = Vec::new();
    for dir in &dirs {
        let text = text_of(dir)?;
        if text.contains(separator) {
            return Err(LunaError::new(
                ErrorCode::InvalidPath,
                format!("{text} contains {separator}, which separates the folders of PATH"),
            )
            .with_details(json!({ "path": text })));
        }
        dir_texts.push(text);
    }
    let mut path = OsString::from(dir_texts.join(separator));
    if let Some(original) = original_path.filter(|p| !p.is_empty()) {
        path.push(separator);
        path.push(original);
    }
    let mut vars: Vec<(OsString, OsString)> = vec![("PATH".into(), path)];
    for (name, value) in php_environment(home, &tools.php.version)? {
        vars.push((name.into(), value));
    }
    vars.push(("LUNA_PROJECT".into(), project_id.into()));

    let folder_text = text_of(folder)?;
    let say = banner(tools);
    match os {
        Os::Windows => Ok(TerminalPlan {
            files,
            command: OpenCommand {
                program: PathBuf::from("powershell.exe"),
                args: vec![
                    "-NoLogo".to_owned(),
                    "-NoExit".to_owned(),
                    "-Command".to_owned(),
                    format!("Write-Host '{say}'"),
                ],
                cwd: Some(folder.to_path_buf()),
                env: vars,
                new_console: true,
            },
        }),
        _ => {
            let extension = if os == Os::Macos { "command" } else { "sh" };
            let script = home
                .run_dir()
                .join("terminals")
                .join(format!("{project_id}.{extension}"));
            let mut content = String::from("#!/bin/sh\n");
            content.push_str(&format!(
                "# Generated by lunaRun for the project {project_id}. It is overwritten each time a terminal is opened.\n"
            ));
            let dir_args: Vec<String> = dir_texts.iter().map(|d| sh_quote(d)).collect();
            content.push_str(&format!("export PATH={}:\"$PATH\"\n", dir_args.join(":")));
            for (name, value) in vars.iter().skip(1) {
                content.push_str(&format!(
                    "export {}={}\n",
                    name.to_string_lossy(),
                    sh_quote(&value.to_string_lossy())
                ));
            }
            content.push_str(&format!("cd {} || exit 1\n", sh_quote(&folder_text)));
            content.push_str(&format!("echo {}\n", sh_quote(&say)));
            content.push_str("exec \"${SHELL:-/bin/sh}\"\n");
            let script_text = text_of(&script)?;
            files.push(GeneratedFile {
                path: script,
                content,
                executable: true,
            });
            let command = if os == Os::Macos {
                OpenCommand {
                    program: PathBuf::from("open"),
                    args: vec!["-a".to_owned(), "Terminal".to_owned(), script_text],
                    cwd: None,
                    env: Vec::new(),
                    new_console: false,
                }
            } else {
                let found = (env.var)("PATH").and_then(|paths| {
                    LINUX_TERMINALS.iter().find_map(|(name, flag)| {
                        paths
                            .to_string_lossy()
                            .split(':')
                            .filter(|dir| !dir.is_empty())
                            .map(|dir| PathBuf::from(dir).join(name))
                            .find(|candidate| (env.exists)(candidate))
                            .map(|program| (program, *flag))
                    })
                });
                let Some((program, flag)) = found else {
                    return Err(LunaError::new(
                        ErrorCode::NotFound,
                        "no terminal program was found (x-terminal-emulator, gnome-terminal, konsole, xterm)",
                    )
                    .with_details(json!({ "target": "terminal" })));
                };
                OpenCommand {
                    program,
                    args: vec![flag.to_owned(), script_text],
                    cwd: Some(folder.to_path_buf()),
                    env: Vec::new(),
                    new_console: false,
                }
            };
            Ok(TerminalPlan { files, command })
        }
    }
}

/// Write the files of a plan: atomically, scripts made runnable.
///
/// # Errors
/// I/O errors.
pub fn write_files(files: &[GeneratedFile]) -> Result<()> {
    for file in files {
        write_atomic(&file.path, file.content.as_bytes())?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file.path, std::fs::Permissions::from_mode(0o755))
                .map_err(LunaError::from)?;
        }
    }
    Ok(())
}

/// The installed runtime with the highest version, `None` when there is none.
fn newest<'a>(installed: &'a [InstalledRuntime], runtime: &str) -> Option<&'a InstalledRuntime> {
    installed
        .iter()
        .filter(|r| r.runtime == runtime)
        .max_by_key(|r| RuntimeVersion::parse(&r.version))
}

/// What a project's terminal gets: the PHP it resolves to (which must be installed), its Node when one is installed, and the
/// newest Composer.
fn toolchain_for<'a>(
    sites: &SiteManager,
    project: &Project,
    installed: &'a [InstalledRuntime],
) -> Result<Toolchain<'a>> {
    let (_report, resolution) = sites.evaluate(project)?;
    let installed_version = |chosen: Option<&crate::projects::Recommendation>| {
        chosen
            .filter(|c| c.installed && !c.needs_install)
            .map(|c| c.version.clone())
    };
    let php_version = installed_version(resolution.php.recommended.as_ref()).ok_or_else(|| {
        LunaError::new(
            ErrorCode::NoSite,
            "a terminal needs the PHP of the project, and it is not installed (see the checklist of the project)",
        )
        .with_details(json!({ "projectId": project.id, "reason": "php" }))
    })?;
    let php = installed
        .iter()
        .find(|r| r.runtime == "php" && r.version == php_version)
        .ok_or_else(|| LunaError::internal("the PHP of the project is not installed"))?;
    let node = resolution
        .node
        .as_ref()
        .and_then(|n| installed_version(n.recommended.as_ref()))
        .and_then(|version| {
            installed
                .iter()
                .find(|r| r.runtime == "node" && r.version == version)
        });
    Ok(Toolchain {
        php,
        node,
        composer: newest(installed, "composer"),
    })
}

/// Everything a terminal for project `id` needs, from what is installed now. Nothing is written or started.
///
/// # Errors
/// As [`open`].
pub fn prepare(
    home: &LunaHome,
    os: Os,
    store: &ProjectStore,
    sites: &SiteManager,
    id: &str,
) -> Result<TerminalPlan> {
    let project = store.get(id)?;
    let installed = list_installed(home)?;
    let tools = toolchain_for(sites, &project, &installed)?;
    plan(
        home,
        &project.id,
        Path::new(&project.path),
        &tools,
        std::env::var_os("PATH").as_deref(),
        &OpenEnv {
            os,
            var: &|name| std::env::var_os(name),
            exists: &|path| path.exists(),
        },
    )
}

/// Open a terminal for project `id`: write what it needs inside LUNA_HOME and start it.
///
/// # Errors
/// `NOT_FOUND` (no such project, its folder is gone, or no terminal program is found: `details.target` `terminal`),
/// `NO_SITE` (its PHP is not installed), `INVALID_PATH`, and I/O errors.
pub fn open(
    home: &LunaHome,
    os: Os,
    store: &ProjectStore,
    sites: &SiteManager,
    id: &str,
) -> Result<()> {
    let planned = prepare(home, os, store, sites, id)?;
    write_files(&planned.files)?;
    spawn(&planned.command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    use crate::runtimes::InstallReceipt;

    fn runtime(name: &str, version: &str, dir: &str) -> InstalledRuntime {
        InstalledRuntime {
            runtime: name.to_owned(),
            version: version.to_owned(),
            dir: PathBuf::from(dir),
            receipt: InstallReceipt::new(
                name,
                version,
                Os::Windows,
                crate::diagnostics::Arch::X86_64,
                false,
                &[],
            ),
        }
    }

    struct World {
        vars: HashMap<&'static str, OsString>,
        files: HashSet<PathBuf>,
    }

    impl World {
        fn new(vars: &[(&'static str, &str)], files: &[&str]) -> Self {
            Self {
                vars: vars.iter().map(|(k, v)| (*k, OsString::from(v))).collect(),
                files: files.iter().map(PathBuf::from).collect(),
            }
        }

        fn plan(
            &self,
            os: Os,
            home: &LunaHome,
            folder: &str,
            tools: &Toolchain<'_>,
            original: Option<&str>,
        ) -> Result<TerminalPlan> {
            plan(
                home,
                "4b1e0c",
                Path::new(folder),
                tools,
                original.map(OsStr::new),
                &OpenEnv {
                    os,
                    var: &|name| self.vars.get(name).cloned(),
                    exists: &|path| self.files.contains(path),
                },
            )
        }
    }

    fn home() -> (tempfile::TempDir, LunaHome) {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        (tmp, home)
    }

    fn var(plan: &TerminalPlan, name: &str) -> String {
        plan.command
            .env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    #[test]
    fn the_windows_terminal_is_powershell_in_the_folder_with_the_environment_of_the_project() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", r"C:\Users\Ada\lüna\php\8.3.14");
        let node = runtime("node", "22.11.0", r"C:\Users\Ada\lüna\node\22.11.0");
        let composer = runtime("composer", "2.8.4", r"C:\Users\Ada\lüna\composer\2.8.4");
        let folder = r"D:\Work\mon projet é";
        let world = World::new(&[], &[folder]);
        let tools = Toolchain {
            php: &php,
            node: Some(&node),
            composer: Some(&composer),
        };

        let plan = world
            .plan(
                Os::Windows,
                &home,
                folder,
                &tools,
                Some(r"C:\Windows;C:\Tools"),
            )
            .unwrap();

        let command = &plan.command;
        assert_eq!(command.program, PathBuf::from("powershell.exe"));
        assert_eq!(command.args[..3], ["-NoLogo", "-NoExit", "-Command"]);
        assert_eq!(
            command.args[3],
            "Write-Host 'lunaRun: PHP 8.3.14, Node.js 22.11.0, Composer 2.8.4'"
        );
        assert_eq!(command.cwd.as_deref(), Some(Path::new(folder)));
        assert!(command.new_console);
        // PATH: PHP, the composer shim, Node, then the original, separated by `;`.
        let shim = shim_dir(&home, "8.3.14");
        assert_eq!(
            var(&plan, "PATH"),
            format!(
                r"{};{};{};C:\Windows;C:\Tools",
                php.dir.display(),
                shim.display(),
                node.dir.display()
            )
        );
        assert_eq!(var(&plan, "LUNA_PROJECT"), "4b1e0c");
        assert!(
            var(&plan, "PHPRC").contains("8.3.14"),
            "{}",
            var(&plan, "PHPRC")
        );
        assert!(var(&plan, "PHP_INI_SCAN_DIR").contains("conf.d"));
        // The shim is the only file, and it is inside LUNA_HOME.
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path, shim.join("composer.bat"));
        assert!(plan.files[0].path.starts_with(home.root()));
    }

    #[test]
    fn the_composer_shim_runs_composer_with_that_php_and_escapes_what_a_shell_reads() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", "/100%/php");
        let composer = runtime("composer", "2.8.4", "/c");
        let world = World::new(&[], &["/f"]);
        let tools = Toolchain {
            php: &php,
            node: None,
            composer: Some(&composer),
        };
        let windows = world.plan(Os::Windows, &home, "/f", &tools, None).unwrap();
        let bat = &windows.files[0].content;
        assert!(
            bat.starts_with("@echo off\r\nrem Generated by lunaRun"),
            "{bat}"
        );
        // Paths are built the way the adapters build them: a `\` separates folders only on a Windows machine.
        let percent_doubled = |path: PathBuf| path.display().to_string().replace('%', "%%");
        let expected = format!(
            "\"{}\" \"{}\" %*",
            percent_doubled(php.dir.join("php.exe")),
            percent_doubled(composer.dir.join("composer.phar"))
        );
        assert!(bat.contains(&expected), "{bat}");
        assert!(bat.contains("%%"), "{bat}");
        assert!(bat.ends_with("\r\n"));

        // The Unix shim, built directly: the folder of LUNA_HOME is the machine's own, and a Windows drive letter has a `:`.
        let sh = composer_shim(
            Os::Linux,
            &home,
            "8.3.14",
            Path::new("/opt/it's/php/php"),
            Path::new("/opt/composer/composer.phar"),
        )
        .unwrap();
        assert_eq!(sh.path, shim_dir(&home, "8.3.14").join("composer"));
        assert!(sh.executable);
        assert_eq!(
            sh.content,
            "#!/bin/sh\n# Generated by lunaRun: runs Composer with PHP 8.3.14.\nexec '/opt/it'\\''s/php/php' '/opt/composer/composer.phar' \"$@\"\n"
        );
    }

    #[test]
    fn a_shim_inside_lunar_home_names_its_programs_relative_to_itself_so_no_accented_letter_is_in_the_file()
     {
        // A batch file is read in the console's code page: `ü` in a profile or home folder would not be found.
        let (_tmp, home) = home();
        let php_dir = home.root().join("runtimes").join("php").join("8.3.14");
        let composer_dir = home.root().join("runtimes").join("composer").join("2.8.4");
        let shim = composer_shim(
            Os::Windows,
            &home,
            "8.3.14",
            &php_dir.join("php.exe"),
            &composer_dir.join("composer.phar"),
        )
        .unwrap();

        assert_eq!(
            shim.content,
            "@echo off\r\nrem Generated by lunaRun: runs Composer with PHP 8.3.14.\r\n\
             \"%~dp0..\\..\\..\\runtimes\\php\\8.3.14\\php.exe\" \
             \"%~dp0..\\..\\..\\runtimes\\composer\\2.8.4\\composer.phar\" %*\r\n"
        );
        assert!(shim.content.is_ascii(), "{}", shim.content);
        // The shim folder really is three folders below LUNA_HOME, which is what the three `..` climb.
        assert_eq!(
            shim.path
                .parent()
                .unwrap()
                .strip_prefix(home.root())
                .unwrap()
                .components()
                .count(),
            3
        );
    }

    #[test]
    fn without_node_or_composer_the_path_has_only_what_is_there() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", "/opt/php");
        let world = World::new(&[("PATH", "/usr/bin")], &["/f", "/usr/bin/konsole"]);
        let tools = Toolchain {
            php: &php,
            node: None,
            composer: None,
        };

        let linux = world
            .plan(Os::Linux, &home, "/f", &tools, Some("/usr/bin:/bin"))
            .unwrap();

        // A Unix terminal gets its environment from the script, not from the process.
        assert!(linux.command.env.is_empty());
        assert!(
            linux
                .files
                .iter()
                .all(|f| f.path.file_name() != Some(OsStr::new("composer")))
        );
        let script = linux
            .files
            .iter()
            .find(|f| f.path.extension() == Some(OsStr::new("sh")))
            .unwrap();
        assert!(
            script
                .content
                .contains("export PATH='/opt/php':\"$PATH\"\n"),
            "{}",
            script.content
        );
        assert!(script.content.contains("echo 'lunaRun: PHP 8.3.14'\n"));
        assert!(!script.content.contains("Node.js"));
    }

    #[test]
    fn the_mac_terminal_opens_a_script_that_sets_the_environment_and_goes_to_the_folder() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", "/Users/ada/lü na/php");
        let node = runtime("node", "22.11.0", "/Users/ada/lü na/node");
        let world = World::new(&[], &["/Users/ada/projets é/shop"]);
        let tools = Toolchain {
            php: &php,
            node: Some(&node),
            composer: None,
        };

        let mac = world
            .plan(Os::Macos, &home, "/Users/ada/projets é/shop", &tools, None)
            .unwrap();

        let script = &mac.files[0];
        assert_eq!(
            script.path,
            home.run_dir().join("terminals").join("4b1e0c.command")
        );
        assert!(script.executable);
        let lines: Vec<&str> = script.content.lines().collect();
        assert_eq!(lines[0], "#!/bin/sh");
        assert!(
            script.content.contains(&format!(
                "export PATH='/Users/ada/lü na/php':'{}':\"$PATH\"\n",
                node.dir.join("bin").display()
            )),
            "{}",
            script.content
        );
        assert!(script.content.contains("export LUNA_PROJECT='4b1e0c'\n"));
        assert!(
            script
                .content
                .contains("cd '/Users/ada/projets é/shop' || exit 1\n")
        );
        assert!(script.content.ends_with("exec \"${SHELL:-/bin/sh}\"\n"));
        assert_eq!(
            mac.command,
            OpenCommand {
                program: PathBuf::from("open"),
                args: vec![
                    "-a".into(),
                    "Terminal".into(),
                    script.path.display().to_string()
                ],
                cwd: None,
                env: Vec::new(),
                new_console: false,
            }
        );
    }

    #[test]
    fn linux_uses_the_first_terminal_program_that_is_installed() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", "/opt/php");
        let tools = Toolchain {
            php: &php,
            node: None,
            composer: None,
        };
        let path = "/usr/local/bin:/usr/bin";
        for (installed, flag) in [
            (vec!["/usr/bin/x-terminal-emulator", "/usr/bin/xterm"], "-e"),
            (vec!["/usr/bin/gnome-terminal", "/usr/bin/konsole"], "--"),
            (vec!["/usr/bin/konsole", "/usr/bin/xterm"], "-e"),
            (vec!["/usr/local/bin/xterm"], "-e"),
        ] {
            let mut files = vec!["/srv/p"];
            files.extend(installed.iter().copied());
            let world = World::new(&[("PATH", path)], &files);
            let linux = world
                .plan(Os::Linux, &home, "/srv/p", &tools, None)
                .unwrap();
            assert_eq!(
                linux.command.program,
                PathBuf::from(installed[0]),
                "{installed:?}"
            );
            assert_eq!(linux.command.args[0], flag);
            assert_eq!(
                linux.command.args[1],
                home.run_dir()
                    .join("terminals")
                    .join("4b1e0c.sh")
                    .display()
                    .to_string()
            );
            assert_eq!(linux.command.cwd.as_deref(), Some(Path::new("/srv/p")));
        }
    }

    #[test]
    fn no_terminal_program_is_an_answer_that_names_what_is_missing() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", "/opt/php");
        let tools = Toolchain {
            php: &php,
            node: None,
            composer: None,
        };
        let world = World::new(&[("PATH", "/usr/bin")], &["/srv/p"]);
        let err = world
            .plan(Os::Linux, &home, "/srv/p", &tools, None)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.details.unwrap()["target"], "terminal");
    }

    #[test]
    fn a_folder_that_is_gone_unsafe_text_and_an_unknown_system_are_refused() {
        let (_tmp, home) = home();
        let php = runtime("php", "8.3.14", "/opt/php");
        let tools = Toolchain {
            php: &php,
            node: None,
            composer: None,
        };
        let world = World::new(&[("PATH", "/usr/bin")], &["/usr/bin/xterm"]);
        assert_eq!(
            world
                .plan(Os::Linux, &home, "/gone", &tools, None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );

        let world = World::new(&[("PATH", "/usr/bin")], &["/f", "/usr/bin/xterm"]);
        let evil = runtime("php", "8.3.14'; rm -rf ~; '", "/opt/php");
        let bad = Toolchain {
            php: &evil,
            node: None,
            composer: None,
        };
        let err = world.plan(Os::Linux, &home, "/f", &bad, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(err.details.unwrap()["field"], "PHP version");

        // A folder of PATH that contains the separator cannot be one entry of PATH.
        let odd = runtime("php", "8.3.14", "/opt/a:b/php");
        let tools = Toolchain {
            php: &odd,
            node: None,
            composer: None,
        };
        assert_eq!(
            world
                .plan(Os::Linux, &home, "/f", &tools, None)
                .unwrap_err()
                .code,
            ErrorCode::InvalidPath
        );
        assert!(world.plan(Os::Unknown, &home, "/f", &tools, None).is_err());
    }

    #[test]
    fn the_newest_composer_is_the_one_the_terminal_gets() {
        let installed = vec![
            runtime("composer", "2.7.9", "/c/2.7.9"),
            runtime("composer", "2.10.1", "/c/2.10.1"),
            runtime("composer", "2.8.4", "/c/2.8.4"),
            runtime("php", "9.0.0", "/p"),
        ];
        assert_eq!(newest(&installed, "composer").unwrap().version, "2.10.1");
        assert!(newest(&installed, "node").is_none());
    }
}
