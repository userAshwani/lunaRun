//! Looking after the database lunaRun runs for a project: does the database the `.env` names exist, and create it
//! (PROJECT-DETECTION section 4 "Create database"; D-041).
//!
//! It talks to the server the way a person at a prompt would: with the `mysql` or `mariadb` **client of the installed
//! runtime**, started with an argument array (golden rule 6), as `root` over TCP to `127.0.0.1` and the port lunaRun gave
//! the service, with **no password** (lunaRun's databases accept this computer only, D-027) and `--no-defaults`, so no
//! option file of the user's changes what is connected to. The `.env` password is never read (golden rule 7).
//!
//! The only text that reaches SQL is the database name, and it is validated first ([`is_valid_name`]): letters, digits, `_`,
//! `$` and `-`, at most 64 characters, so it cannot end a statement or a quote whatever the `.env` says.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use ts_rs::TS;

use crate::error::{ErrorCode, LunaError, Result};
use crate::runtimes::adapter::run_program_raw;

/// How long the client may take: it connects to this computer, so a slow answer is a stuck server.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Where the database a project names stands, for the checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DatabaseStatus {
    /// The project expects no database lunaRun can run (none named, SQLite, not installed).
    NotApplicable,
    /// The `.env` file names no `DB_DATABASE`.
    NoName,
    /// The name has characters lunaRun will not put into a statement.
    InvalidName,
    /// The database server is not running, so nothing can be asked of it.
    NotRunning,
    Exists,
    Missing,
}

/// What a created database looks like afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseCreated {
    pub name: String,
    /// `false` when it was there already (nothing was changed).
    pub created: bool,
}

/// Whether `name` may be put into a statement: ASCII letters, digits, `_`, `$` and `-`, 1 to 64 characters.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'-'))
}

/// The arguments of one statement. `--no-defaults` must come first.
fn arguments(port: u16, statement: &str) -> Vec<String> {
    vec![
        "--no-defaults".to_owned(),
        "--protocol=TCP".to_owned(),
        "--host=127.0.0.1".to_owned(),
        format!("--port={port}"),
        "--user=root".to_owned(),
        "--batch".to_owned(),
        "--skip-column-names".to_owned(),
        "--connect-timeout=5".to_owned(),
        format!("--execute={statement}"),
    ]
}

fn invalid(name: &str) -> LunaError {
    LunaError::new(
        ErrorCode::InvalidInput,
        format!("{name:?} is not a database name lunaRun will use (letters, digits, _, $ and - only, 64 at most)"),
    )
    .with_details(json!({ "field": "database" }))
}

/// Run one statement and return what it printed.
fn run(client: &Path, runtime: &str, port: u16, statement: &str) -> Result<String> {
    let cwd = client.parent().unwrap_or_else(|| Path::new("."));
    let finished = run_program_raw(runtime, client, &arguments(port, statement), cwd, TIMEOUT)?;
    if finished.success {
        return Ok(finished.output);
    }
    let tail: Vec<&str> = finished.output.lines().rev().take(3).collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    Err(LunaError::new(
        ErrorCode::DatabaseCommandFailed,
        format!("the database client exited with {}", finished.status),
    )
    .with_details(json!({ "tail": tail, "port": port })))
}

/// Does the database `name` exist on the server at `port`?
///
/// # Errors
/// `INVALID_INPUT` for a name that is not valid, `DATABASE_COMMAND_FAILED` when the server refused or answered something
/// unexpected, `RUNTIME_BROKEN` when the client cannot be started.
pub fn exists(client: &Path, runtime: &str, port: u16, name: &str) -> Result<bool> {
    if !is_valid_name(name) {
        return Err(invalid(name));
    }
    let statement =
        format!("SELECT COUNT(*) FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = '{name}'");
    let answer = run(client, runtime, port, &statement)?;
    parse_exists(&answer).ok_or_else(|| {
        LunaError::new(
            ErrorCode::DatabaseCommandFailed,
            "the database client gave an answer lunaRun does not understand",
        )
        .with_details(json!({ "tail": last_lines(&answer) }))
    })
}

/// The answer of the count query: the line that is exactly `0` or `1`. The client writes its warnings (MariaDB's
/// "option --ssl-verify-server-cert is disabled, because of an insecure passwordless login") to **stderr**, which
/// arrives after the answer in the one text both streams are read into, so neither the first nor the last line is it.
fn parse_exists(output: &str) -> Option<bool> {
    output.lines().map(str::trim).find_map(|line| match line {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    })
}

/// The end of what a program said, for an error's details.
fn last_lines(output: &str) -> String {
    let lines: Vec<&str> = output.lines().rev().take(3).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(
        "
",
    )
}

/// Create the database `name` (utf8mb4, `utf8mb4_unicode_ci`, which MySQL and MariaDB both have) unless it exists.
/// Returns whether it was created.
///
/// # Errors
/// As [`exists`].
pub fn create(client: &Path, runtime: &str, port: u16, name: &str) -> Result<bool> {
    if exists(client, runtime, port, name)? {
        return Ok(false);
    }
    let statement = format!(
        "CREATE DATABASE IF NOT EXISTS `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
    );
    run(client, runtime, port, &statement)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_plain_or_it_is_refused() {
        for good in ["shop", "my_shop", "shop-2", "Shop$1", "a", &"x".repeat(64)] {
            assert!(is_valid_name(good), "{good}");
        }
        for bad in [
            "",
            "my shop",
            "shop;drop",
            "a`b",
            "a'b",
            "shop\"",
            "café",
            "a.b",
            "a/b",
            "a\\b",
            "shop\n",
            &"x".repeat(65),
        ] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_answer_is_the_line_that_is_a_count_whatever_warnings_come_before_or_after_it() {
        assert_eq!(
            parse_exists(
                "1
"
            ),
            Some(true)
        );
        assert_eq!(parse_exists("0"), Some(false));
        assert_eq!(
            parse_exists(
                "1
"
            ),
            Some(true)
        );
        let warning = "WARNING: option --ssl-verify-server-cert is disabled, because of an insecure passwordless login.";
        // Before it (a client that writes both to one stream) and after it (stdout, then stderr, as lunaRun reads them).
        assert_eq!(
            parse_exists(&format!(
                "{warning}
0
"
            )),
            Some(false)
        );
        assert_eq!(
            parse_exists(&format!(
                "0
{warning}"
            )),
            Some(false)
        );
        assert_eq!(
            parse_exists(&format!(
                "1
{warning}
"
            )),
            Some(true)
        );
        assert_eq!(parse_exists(""), None);
        assert_eq!(
            parse_exists(
                "
  
"
            ),
            None
        );
        assert_eq!(parse_exists("2"), None);
        assert_eq!(parse_exists(warning), None);
        assert_eq!(parse_exists("10"), None);
    }

    #[test]
    fn the_arguments_are_an_array_with_no_defaults_first_and_no_password() {
        let args = arguments(3307, "SELECT 1");
        assert_eq!(args[0], "--no-defaults");
        assert_eq!(
            args,
            [
                "--no-defaults",
                "--protocol=TCP",
                "--host=127.0.0.1",
                "--port=3307",
                "--user=root",
                "--batch",
                "--skip-column-names",
                "--connect-timeout=5",
                "--execute=SELECT 1",
            ]
        );
        assert!(
            !args
                .iter()
                .any(|a| a.contains("password") || a.starts_with("-p"))
        );
    }

    #[test]
    fn a_name_that_is_not_plain_never_reaches_a_statement() {
        let client = Path::new("does-not-exist");
        for bad in ["a`; DROP DATABASE mysql; --", "x' OR '1'='1", ""] {
            let err = exists(client, "mysql", 3306, bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
            let err = create(client, "mysql", 3306, bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
        }
    }
}
