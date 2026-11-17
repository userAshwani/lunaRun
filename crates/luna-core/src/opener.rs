//! The quick actions that open something outside lunaRun: a project's site in the browser, its folder in the file
//! manager, its folder in the code editor (FR-PJ-09; D-039).
//!
//! [`command_for`] says **which program is run with which arguments** for each system, from what it is told about the
//! system (`OpenEnv`), so every case is tested on any machine. [`spawn`] runs it, detached, with **an argument array and
//! no shell** (golden rule 6). The programs are the system's own openers (`explorer`, `open`, `xdg-open`, `rundll32`)
//! and Visual Studio Code; the only values passed are a folder lunaRun itself stored for the project and an address it
//! built itself (`http://127.0.0.1:<port>`), so nothing the page sends is ever run or opened.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::json;
use ts_rs::TS;

use crate::diagnostics::Os;
use crate::error::{ErrorCode, LunaError, Result};

/// What to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OpenTarget {
    /// The project's site, `http://127.0.0.1:<port>`, in the default browser.
    Browser,
    /// The project's folder in the file manager.
    Folder,
    /// The project's folder in Visual Studio Code, when it is installed.
    Editor,
}

/// A program and its arguments: what [`spawn`] runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// What [`command_for`] needs to know about the system, so tests can say it.
pub struct OpenEnv<'a> {
    pub os: Os,
    /// An environment variable.
    pub var: &'a dyn Fn(&str) -> Option<OsString>,
    /// Does this path exist?
    pub exists: &'a dyn Fn(&Path) -> bool,
}

fn command(program: impl Into<PathBuf>, args: &[&str]) -> OpenCommand {
    OpenCommand {
        program: program.into(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    }
}

fn path_arg(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        LunaError::new(
            ErrorCode::InvalidPath,
            format!("{} cannot be passed to another program", path.display()),
        )
        .with_details(json!({ "path": path.display().to_string() }))
    })
}

/// Only an address lunaRun built for a site is ever opened: `http://127.0.0.1:` and a port.
fn check_address(address: &str) -> Result<()> {
    let port = address.strip_prefix("http://127.0.0.1:").unwrap_or("");
    if !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(());
    }
    Err(LunaError::new(
        ErrorCode::InvalidInput,
        format!("{address} is not the address of a site"),
    )
    .with_details(json!({ "field": "address" })))
}

/// The first Visual Studio Code that is installed in the usual places of `env.os`.
fn editor_program(env: &OpenEnv<'_>) -> Option<EditorLaunch> {
    match env.os {
        Os::Windows => ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|name| (env.var)(name))
            .flat_map(|base| {
                let base = PathBuf::from(base);
                [
                    base.join("Programs")
                        .join("Microsoft VS Code")
                        .join("Code.exe"),
                    base.join("Microsoft VS Code").join("Code.exe"),
                ]
            })
            .find(|candidate| (env.exists)(candidate))
            .map(EditorLaunch::Direct),
        Os::Macos => {
            let app = "Visual Studio Code.app";
            let mut places = vec![PathBuf::from("/Applications").join(app)];
            if let Some(home) = (env.var)("HOME") {
                places.push(PathBuf::from(home).join("Applications").join(app));
            }
            places
                .into_iter()
                .find(|place| (env.exists)(place))
                .map(|_| EditorLaunch::MacApp)
        }
        // `PATH` of a Unix system is separated by `:` (not the host's separator: this branch is for Linux, and tests
        // run it on any machine).
        Os::Linux | Os::Unknown => (env.var)("PATH").and_then(|paths| {
            paths
                .to_string_lossy()
                .split(':')
                .filter(|dir| !dir.is_empty())
                .map(|dir| PathBuf::from(dir).join("code"))
                .find(|candidate| (env.exists)(candidate))
                .map(EditorLaunch::Direct)
        }),
    }
}

enum EditorLaunch {
    /// Run this program with the folder as its only argument.
    Direct(PathBuf),
    /// macOS: `open -a "Visual Studio Code" <folder>`.
    MacApp,
}

/// The program to run for `target`.
///
/// - `address` is the site address (needed for [`OpenTarget::Browser`]), `folder` the project's folder.
/// - Windows: `rundll32 url.dll,FileProtocolHandler <url>`, `explorer <folder>`, `Code.exe <folder>`; macOS: `open`;
///   Linux: `xdg-open`, and `code` when it is on `PATH`.
///
/// # Errors
/// `INVALID_INPUT` for an address that is not a site's, `NOT_FOUND` for a folder that is not there or (with
/// `details.target` `editor`) when no Visual Studio Code is installed, `INVALID_PATH` for a path that is not text.
pub fn command_for(
    target: OpenTarget,
    address: Option<&str>,
    folder: &Path,
    env: &OpenEnv<'_>,
) -> Result<OpenCommand> {
    if env.os == Os::Unknown {
        return Err(LunaError::internal(
            "opening something is not supported on this system",
        ));
    }
    match target {
        OpenTarget::Browser => {
            let address = address.ok_or_else(|| {
                LunaError::new(ErrorCode::NoSite, "the project has no site to open")
            })?;
            check_address(address)?;
            Ok(match env.os {
                Os::Windows => command("rundll32.exe", &["url.dll,FileProtocolHandler", address]),
                Os::Macos => command("open", &[address]),
                _ => command("xdg-open", &[address]),
            })
        }
        OpenTarget::Folder | OpenTarget::Editor => {
            if !(env.exists)(folder) {
                return Err(LunaError::new(
                    ErrorCode::NotFound,
                    format!("the project folder {} is not there", folder.display()),
                )
                .with_details(json!({ "path": folder.display().to_string() })));
            }
            let folder = path_arg(folder)?;
            if target == OpenTarget::Folder {
                return Ok(match env.os {
                    Os::Windows => command("explorer.exe", &[&folder]),
                    Os::Macos => command("open", &[&folder]),
                    _ => command("xdg-open", &[&folder]),
                });
            }
            match editor_program(env) {
                Some(EditorLaunch::Direct(program)) => Ok(command(program, &[&folder])),
                Some(EditorLaunch::MacApp) => {
                    Ok(command("open", &["-a", "Visual Studio Code", &folder]))
                }
                None => Err(LunaError::new(
                    ErrorCode::NotFound,
                    "Visual Studio Code was not found in the usual places",
                )
                .with_details(json!({ "target": "editor" }))),
            }
        }
    }
}

/// [`command_for`] with this computer's environment and file system.
///
/// # Errors
/// As [`command_for`].
pub fn command_here(
    target: OpenTarget,
    address: Option<&str>,
    folder: &Path,
) -> Result<OpenCommand> {
    command_for(
        target,
        address,
        folder,
        &OpenEnv {
            os: Os::current(),
            var: &|name| std::env::var_os(name),
            exists: &|path| path.exists(),
        },
    )
}

/// Run `command` and do not wait for it: the program is a window the user is meant to keep using. Arguments are passed
/// as an array, never through a shell.
///
/// # Errors
/// I/O errors from starting it (`NOT_FOUND` when the program is missing).
pub fn spawn(command: &OpenCommand) -> Result<()> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            let shown = command.program.display().to_string();
            let mut error = LunaError::from(err);
            error.message = format!("{shown} could not be started: {}", error.message);
            error.with_details(json!({ "program": shown }))
        })?;
    // Collect it when it ends, so it does not stay a zombie on macOS and Linux.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

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

        fn run(
            &self,
            os: Os,
            target: OpenTarget,
            address: Option<&str>,
            folder: &str,
        ) -> Result<OpenCommand> {
            command_for(
                target,
                address,
                Path::new(folder),
                &OpenEnv {
                    os,
                    var: &|name| self.vars.get(name).cloned(),
                    exists: &|path| self.files.contains(path),
                },
            )
        }
    }

    const FOLDER_WIN: &str = r"D:\Work\mon projet é";

    #[test]
    fn the_browser_opens_the_address_of_the_site_through_the_systems_own_opener() {
        let world = World::new(&[], &[]);
        let url = Some("http://127.0.0.1:8101");
        assert_eq!(
            world
                .run(Os::Windows, OpenTarget::Browser, url, "x")
                .unwrap(),
            command(
                "rundll32.exe",
                &["url.dll,FileProtocolHandler", "http://127.0.0.1:8101"]
            )
        );
        assert_eq!(
            world.run(Os::Macos, OpenTarget::Browser, url, "x").unwrap(),
            command("open", &["http://127.0.0.1:8101"])
        );
        assert_eq!(
            world.run(Os::Linux, OpenTarget::Browser, url, "x").unwrap(),
            command("xdg-open", &["http://127.0.0.1:8101"])
        );
    }

    #[test]
    fn only_the_address_of_a_site_is_ever_opened() {
        let world = World::new(&[], &[]);
        for bad in [
            "https://example.com",
            "http://example.com:80",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:80a",
            "http://127.0.0.1:123456",
            "http://127.0.0.1:80/../x",
            "file:///C:/Windows/System32/calc.exe",
            "calc.exe",
            "http://localhost:8101",
            "",
        ] {
            let err = world
                .run(Os::Windows, OpenTarget::Browser, Some(bad), "x")
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
        }
        let err = world
            .run(Os::Windows, OpenTarget::Browser, None, "x")
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NoSite);
    }

    #[test]
    fn the_folder_opens_in_the_file_manager_and_must_be_there() {
        let world = World::new(&[], &[FOLDER_WIN, "/home/a b/é", "/Users/a/é"]);
        assert_eq!(
            world
                .run(Os::Windows, OpenTarget::Folder, None, FOLDER_WIN)
                .unwrap(),
            command("explorer.exe", &[FOLDER_WIN]),
            "a path with a space and an accented letter is one argument"
        );
        assert_eq!(
            world
                .run(Os::Macos, OpenTarget::Folder, None, "/Users/a/é")
                .unwrap(),
            command("open", &["/Users/a/é"])
        );
        assert_eq!(
            world
                .run(Os::Linux, OpenTarget::Folder, None, "/home/a b/é")
                .unwrap(),
            command("xdg-open", &["/home/a b/é"])
        );
        let err = world
            .run(Os::Windows, OpenTarget::Folder, None, r"D:\gone")
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn the_editor_is_visual_studio_code_where_it_is_installed() {
        let local = r"C:\Users\Ada\AppData\Local";
        let code = r"C:\Users\Ada\AppData\Local\Programs\Microsoft VS Code\Code.exe";
        let world = World::new(&[("LOCALAPPDATA", local)], &[FOLDER_WIN, code]);
        assert_eq!(
            world
                .run(Os::Windows, OpenTarget::Editor, None, FOLDER_WIN)
                .unwrap(),
            command(code, &[FOLDER_WIN])
        );

        // The machine-wide install is found too, when the user one is not there.
        let program_files = r"C:\Program Files";
        let machine = r"C:\Program Files\Microsoft VS Code\Code.exe";
        let world = World::new(
            &[("LOCALAPPDATA", local), ("ProgramFiles", program_files)],
            &[FOLDER_WIN, machine],
        );
        assert_eq!(
            world
                .run(Os::Windows, OpenTarget::Editor, None, FOLDER_WIN)
                .unwrap()
                .program,
            PathBuf::from(machine)
        );

        let mac = World::new(
            &[("HOME", "/Users/ada")],
            &[
                "/Users/ada/p",
                "/Users/ada/Applications/Visual Studio Code.app",
            ],
        );
        assert_eq!(
            mac.run(Os::Macos, OpenTarget::Editor, None, "/Users/ada/p")
                .unwrap(),
            command("open", &["-a", "Visual Studio Code", "/Users/ada/p"])
        );

        let linux = World::new(
            &[("PATH", "/usr/bin:/opt/vs/bin")],
            &["/srv/p", "/opt/vs/bin/code"],
        );
        assert_eq!(
            linux
                .run(Os::Linux, OpenTarget::Editor, None, "/srv/p")
                .unwrap(),
            command("/opt/vs/bin/code", &["/srv/p"])
        );
    }

    #[test]
    fn no_editor_is_a_plain_answer_that_names_what_is_missing() {
        for os in [Os::Windows, Os::Macos, Os::Linux] {
            let world = World::new(&[("PATH", "/usr/bin")], &["/p"]);
            let err = world.run(os, OpenTarget::Editor, None, "/p").unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound, "{os:?}");
            assert_eq!(err.details.unwrap()["target"], "editor");
        }
    }

    #[test]
    fn an_unknown_system_opens_nothing() {
        let world = World::new(&[], &["/p"]);
        assert!(
            world
                .run(Os::Unknown, OpenTarget::Folder, None, "/p")
                .is_err()
        );
    }
}
