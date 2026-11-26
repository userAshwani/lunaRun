//! The one edit lunaRun makes to a project's `.env`: the `DB_PORT` line (FR-PJ-05; docs/PROJECT-DETECTION.md section 4; D-041).
//!
//! This is the **only** place where lunaRun writes into a user's project folder (golden rule 3), so it is built to be
//! predictable:
//!
//! - It only ever runs because the user confirmed a **preview** of the exact line that changes ([`plan_port`] makes the
//!   preview, [`apply_port`] refuses unless the file still gives that same preview).
//! - It changes **one line and nothing else**: every other byte of the file (other lines, their endings, comments, quotes,
//!   a byte order mark, a missing final newline) stays as it was. Inside the line only the value is replaced, so quotes,
//!   an `export ` prefix and a trailing ` # comment` stay too.
//! - A **backup** of the whole file is written first, next to it (`.env.lunarun-backup`, or `-2`, `-3` and so on when one
//!   exists: an older backup is never overwritten). When the backup cannot be written nothing else is.
//! - A file that cannot be edited safely is refused, never guessed at: not UTF-8, a link, too large.
//! - **No secret passes through here**: only the `DB_PORT` line is ever returned or shown, and no value is logged.

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::error::{ErrorCode, IoResultExt, LunaError, Result};
use crate::state::write_atomic_with;

/// The file that is edited.
pub const ENV_FILE: &str = ".env";
/// The name of the backup; `-2`, `-3`, ... when it is taken.
pub const BACKUP_FILE: &str = ".env.lunarun-backup";
/// A `.env` is a few lines. A bigger file is not one, and is not read.
const MAX_ENV_BYTES: u64 = 1024 * 1024;
const KEY: &str = "DB_PORT";
/// What Laravel connects to when `DB_PORT` is not set.
const DEFAULT_PORT: u16 = 3306;

/// The change to `DB_PORT` that would make the `.env` file connect to the database lunaRun runs, as the page previews it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortChange {
    /// The port lunaRun runs the project's database on.
    pub port: u16,
    /// The line number (from 1) of the line that changes, or that the new line will have when there is no `DB_PORT` yet.
    pub line: u32,
    /// The line as it is now, without its line ending; none when the file has no `DB_PORT` and a line is added.
    pub before: Option<String>,
    /// The line as it will be.
    pub after: String,
    /// The name the backup of the whole file will have, next to it.
    pub backup: String,
}

/// What was done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvFixApplied {
    /// The backup, next to the `.env` file.
    pub backup: String,
}

// ---------------------------------------------------------------------------------------------
// The text: pure, byte for byte
// ---------------------------------------------------------------------------------------------

/// One line and the line ending that followed it (`""` for a last line without one).
struct Line<'a> {
    content: &'a str,
    ending: &'a str,
}

fn split(text: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(at) => {
                let (content, ending) = match rest[..at].strip_suffix('\r') {
                    Some(content) => (content, &rest[at - 1..=at]),
                    None => (&rest[..at], &rest[at..=at]),
                };
                lines.push(Line { content, ending });
                rest = &rest[at + 1..];
            }
            None => {
                lines.push(Line {
                    content: rest,
                    ending: "",
                });
                rest = "";
            }
        }
    }
    lines
}

/// An assignment on a line: its key, and where its value is (inside the quotes when it has some).
struct Assignment<'a> {
    key: &'a str,
    value: Range<usize>,
}

/// Where the value of an assignment is, found the way `dotenv::read_keys` reads it (`export` prefix, spaces around `=`,
/// quotes, ` # comment` after an unquoted value), so what is replaced is exactly what was read.
fn assignment(content: &str) -> Option<Assignment<'_>> {
    let mut offset = content.len() - content.trim_start().len();
    let mut body = &content[offset..];
    if body.is_empty() || body.starts_with('#') {
        return None;
    }
    if let Some(rest) = body
        .strip_prefix("export")
        .filter(|r| r.starts_with([' ', '\t']))
    {
        let trimmed = rest.trim_start();
        offset += body.len() - trimmed.len();
        body = trimmed;
    }
    let equals = body.find('=')?;
    let key = body[..equals].trim_end();
    let after_equals = offset + equals + 1;
    let raw = &content[after_equals..];
    let start = after_equals + (raw.len() - raw.trim_start().len());
    let raw = &content[start..];
    for quote in ['"', '\''] {
        if let Some(inner) = raw.strip_prefix(quote) {
            let inner_start = start + 1;
            let end = match inner.find(quote) {
                Some(closing) => inner_start + closing,
                // An unclosed quote: the rest of the line, as dotenv reads it.
                None => inner_start + inner.trim_end().len(),
            };
            return Some(Assignment {
                key,
                value: inner_start..end,
            });
        }
    }
    let cut = raw
        .match_indices('#')
        .find(|(at, _)| *at == 0 || raw[..*at].ends_with([' ', '\t']))
        .map_or(raw.len(), |(at, _)| at);
    let length = raw[..cut].trim_end().len();
    // An empty value is filled in right after the `=`, so a comment after it keeps its space: `DB_PORT=3307 # note`.
    let value = if length == 0 {
        after_equals..after_equals
    } else {
        start..start + length
    };
    Some(Assignment { key, value })
}

/// The new text and what changed in it.
struct Edit {
    text: String,
    line: u32,
    before: Option<String>,
    after: String,
}

/// Make the last effective `DB_PORT` say `port`, or add the line when there is none. `None` when nothing needs to change
/// (it already says `port`, or there is no line and `port` is what Laravel assumes without one).
fn edit(text: &str, port: u16) -> Option<Edit> {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    };
    let lines = split(body);
    let (mut last, mut host, mut connection) = (None, None, None);
    for (index, line) in lines.iter().enumerate() {
        if let Some(found) = assignment(line.content) {
            match found.key {
                KEY => last = Some((index, found)),
                "DB_HOST" => host = Some(index),
                "DB_CONNECTION" => connection = Some(index),
                _ => {}
            }
        }
    }
    let value = port.to_string();
    let rebuild = |replace: usize, with: &str| {
        let mut out = String::with_capacity(body.len() + 16);
        out.push_str(bom);
        for (index, line) in lines.iter().enumerate() {
            out.push_str(if index == replace { with } else { line.content });
            out.push_str(line.ending);
        }
        out
    };

    if let Some((index, found)) = last {
        let content = lines[index].content;
        if content[found.value.clone()].trim().parse::<u16>().ok() == Some(port) {
            return None;
        }
        let after = format!(
            "{}{}{}",
            &content[..found.value.start],
            value,
            &content[found.value.end..]
        );
        return Some(Edit {
            text: rebuild(index, &after),
            line: u32::try_from(index + 1).unwrap_or(u32::MAX),
            before: Some(content.to_owned()),
            after,
        });
    }
    if port == DEFAULT_PORT {
        return None;
    }

    // No DB_PORT: add one after the database settings it belongs with, else at the end. The new line gets the ending of the
    // line before it, or the first ending the file has, or `\n`.
    let after = format!("{KEY}={value}");
    let anchor = host.or(connection).or(lines.len().checked_sub(1));
    let eol = anchor
        .map(|index| lines[index].ending)
        .filter(|ending| !ending.is_empty())
        .or_else(|| lines.iter().map(|l| l.ending).find(|e| !e.is_empty()))
        .unwrap_or("\n");
    let mut out = String::with_capacity(body.len() + after.len() + 2);
    out.push_str(bom);
    let inserted_at = match anchor {
        Some(anchor) => {
            for (index, line) in lines.iter().enumerate() {
                out.push_str(line.content);
                if index == anchor {
                    // The anchor may be a last line with no ending: it gets one, the new line takes its place at the end.
                    out.push_str(if line.ending.is_empty() {
                        eol
                    } else {
                        line.ending
                    });
                    out.push_str(&after);
                    out.push_str(if line.ending.is_empty() {
                        ""
                    } else {
                        line.ending
                    });
                } else {
                    out.push_str(line.ending);
                }
            }
            anchor + 2
        }
        None => {
            out.push_str(&after);
            out.push_str(eol);
            1
        }
    };
    Some(Edit {
        text: out,
        line: u32::try_from(inserted_at).unwrap_or(u32::MAX),
        before: None,
        after,
    })
}

// ---------------------------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------------------------

fn refuse(reason: &str, message: String, path: &Path) -> LunaError {
    LunaError::new(ErrorCode::EnvChanged, message)
        .with_details(json!({ "reason": reason, "path": path.display().to_string() }))
}

/// The `.env` of `folder` as text, or `None` when there is none (the `.env.example` file is never edited).
fn read_env(folder: &Path) -> Result<Option<(PathBuf, String)>> {
    let path = folder.join(ENV_FILE);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).at_path(&path),
    };
    if meta.file_type().is_symlink() {
        return Err(refuse(
            "link",
            format!("{} is a link, which is not edited", path.display()),
            &path,
        ));
    }
    if !meta.is_file() {
        return Err(refuse(
            "notFile",
            format!("{} is not a file", path.display()),
            &path,
        ));
    }
    if meta.len() > MAX_ENV_BYTES {
        return Err(refuse(
            "tooLarge",
            format!("{} is too large to be a .env file", path.display()),
            &path,
        ));
    }
    let bytes = fs::read(&path).at_path(&path)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        refuse(
            "notUtf8",
            format!("{} is not UTF-8 text, so it is not edited", path.display()),
            &path,
        )
    })?;
    Ok(Some((path, text)))
}

/// The first backup name that is not taken.
fn free_backup_name(folder: &Path) -> String {
    (1..1000)
        .map(|n| {
            if n == 1 {
                BACKUP_FILE.to_owned()
            } else {
                format!("{BACKUP_FILE}-{n}")
            }
        })
        .find(|name| fs::symlink_metadata(folder.join(name)).is_err())
        .unwrap_or_else(|| format!("{BACKUP_FILE}-1000"))
}

/// What the page previews for `folder` when the database runs on `port`: the one line that would change and the backup
/// that would be made. `None` when there is nothing to fix: no `.env` file (the `.env.example` is not touched), or its
/// `DB_PORT` already is `port`. Reads only; nothing is written.
///
/// # Errors
/// `ENV_CHANGED` for a `.env` that cannot be edited safely (`details.reason`: `link`, `notFile`, `tooLarge`, `notUtf8`),
/// and I/O errors.
pub fn plan_port(folder: &Path, port: u16) -> Result<Option<PortChange>> {
    let Some((_path, text)) = read_env(folder)? else {
        return Ok(None);
    };
    Ok(edit(&text, port).map(|edit| PortChange {
        port,
        line: edit.line,
        before: edit.before,
        after: edit.after,
        backup: free_backup_name(folder),
    }))
}

/// Make the change the user confirmed: the backup first, then the `.env` file, replaced atomically with its permissions kept.
/// `expected` is what the page showed; when the file no longer gives that same preview nothing is written.
///
/// # Errors
/// `ENV_CHANGED` when the file is not what the preview showed (`details.reason` `changed`) or cannot be edited safely, and
/// I/O errors. When the backup cannot be written, the `.env` file is not touched.
pub fn apply_port(folder: &Path, expected: &PortChange) -> Result<EnvFixApplied> {
    let Some((path, text)) = read_env(folder)? else {
        return Err(refuse(
            "changed",
            format!("there is no .env file in {} any more", folder.display()),
            folder,
        ));
    };
    let Some(change) = edit(&text, expected.port).filter(|c| {
        c.line == expected.line && c.before == expected.before && c.after == expected.after
    }) else {
        return Err(refuse(
            "changed",
            format!("{} changed since the preview", path.display()),
            &path,
        ));
    };

    // The permissions of a `.env` often are the owner's only; the backup and the new file keep them (Unix; on Windows the
    // access rules come from the folder).
    #[cfg(unix)]
    let permissions = fs::metadata(&path).ok().map(|meta| meta.permissions());
    #[cfg(not(unix))]
    let permissions = None;

    let backup = free_backup_name(folder);
    write_atomic_with(&folder.join(&backup), text.as_bytes(), permissions.clone())?;
    write_atomic_with(&path, change.text.as_bytes(), permissions)?;
    Ok(EnvFixApplied { backup })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::dotenv::read_keys;

    fn change(text: &str, port: u16) -> Option<Edit> {
        edit(text, port)
    }

    fn new_text(text: &str, port: u16) -> String {
        change(text, port).expect("a change").text
    }

    #[test]
    fn only_the_port_line_changes_and_every_other_byte_stays() {
        let text =
            "APP_NAME=Shop\nDB_HOST=127.0.0.1\nDB_PORT=3306\nDB_PASSWORD=\"p@ss #1\"\n# end\n";
        let done = change(text, 3307).unwrap();
        assert_eq!(
            done.text,
            "APP_NAME=Shop\nDB_HOST=127.0.0.1\nDB_PORT=3307\nDB_PASSWORD=\"p@ss #1\"\n# end\n"
        );
        assert_eq!(done.line, 3);
        assert_eq!(done.before.as_deref(), Some("DB_PORT=3306"));
        assert_eq!(done.after, "DB_PORT=3307");
    }

    #[test]
    fn quotes_an_export_prefix_spaces_and_a_trailing_comment_stay() {
        assert_eq!(new_text("DB_PORT=\"3306\"\n", 3307), "DB_PORT=\"3307\"\n");
        assert_eq!(new_text("DB_PORT='3306'\n", 3307), "DB_PORT='3307'\n");
        assert_eq!(
            new_text("export DB_PORT = 3306   # mysql\n", 3307),
            "export DB_PORT = 3307   # mysql\n"
        );
        assert_eq!(
            new_text("  DB_PORT=3306\t#x\n", 3307),
            "  DB_PORT=3307\t#x\n"
        );
        // A value that is not a number (a reference) is replaced as a whole.
        assert_eq!(new_text("DB_PORT=${MYSQL_PORT}\n", 3307), "DB_PORT=3307\n");
        // An empty value gets one.
        assert_eq!(new_text("DB_PORT=\nA=1\n", 3307), "DB_PORT=3307\nA=1\n");
        assert_eq!(new_text("DB_PORT= # none\n", 3307), "DB_PORT=3307 # none\n");
    }

    #[test]
    fn line_endings_a_byte_order_mark_and_a_missing_final_newline_are_kept() {
        assert_eq!(
            new_text("A=1\r\nDB_PORT=1\r\nB=2\r\n", 3307),
            "A=1\r\nDB_PORT=3307\r\nB=2\r\n"
        );
        // Mixed endings: the line keeps its own, the others theirs.
        assert_eq!(
            new_text("A=1\nDB_PORT=1\r\nB=2\n", 3307),
            "A=1\nDB_PORT=3307\r\nB=2\n"
        );
        assert_eq!(new_text("\u{feff}DB_PORT=1", 3307), "\u{feff}DB_PORT=3307");
        assert_eq!(new_text("A=1\nDB_PORT=1", 3307), "A=1\nDB_PORT=3307");
    }

    #[test]
    fn the_last_effective_assignment_is_the_one_changed_and_comments_are_left_alone() {
        let text = "# DB_PORT=1111\nDB_PORT=2222\nX=1\nDB_PORT=3333\n# DB_PORT=4444\n";
        let done = change(text, 3307).unwrap();
        assert_eq!(
            done.text,
            "# DB_PORT=1111\nDB_PORT=2222\nX=1\nDB_PORT=3307\n# DB_PORT=4444\n"
        );
        assert_eq!(done.line, 4);
    }

    #[test]
    fn a_port_that_is_already_right_is_not_a_change() {
        assert!(change("DB_PORT=3307\n", 3307).is_none());
        assert!(change("DB_PORT=\"3307\"\n", 3307).is_none());
        assert!(change("DB_PORT = 3307 # x\n", 3307).is_none());
        // Without a line Laravel connects to 3306: nothing to fix when that is the port.
        assert!(change("DB_HOST=127.0.0.1\n", 3306).is_none());
        assert!(change("", 3306).is_none());
        // But another port needs the line.
        assert!(change("DB_HOST=127.0.0.1\n", 3307).is_some());
    }

    #[test]
    fn a_missing_line_is_added_after_the_host_then_the_connection_then_at_the_end() {
        assert_eq!(
            new_text(
                "A=1\nDB_CONNECTION=mysql\nDB_HOST=db\nDB_DATABASE=x\n",
                3307
            ),
            "A=1\nDB_CONNECTION=mysql\nDB_HOST=db\nDB_PORT=3307\nDB_DATABASE=x\n"
        );
        assert_eq!(
            new_text("DB_CONNECTION=mysql\r\nDB_DATABASE=x\r\n", 3307),
            "DB_CONNECTION=mysql\r\nDB_PORT=3307\r\nDB_DATABASE=x\r\n"
        );
        assert_eq!(new_text("A=1\nB=2\n", 3307), "A=1\nB=2\nDB_PORT=3307\n");
        // The anchor is the last line and has no ending: it gets one, and the file still ends the way it did.
        assert_eq!(new_text("DB_HOST=db", 3307), "DB_HOST=db\nDB_PORT=3307");
        assert_eq!(new_text("A=1\r\nB=2", 3307), "A=1\r\nB=2\r\nDB_PORT=3307");
        assert_eq!(new_text("", 3307), "DB_PORT=3307\n");
        let done = change("A=1\nDB_HOST=db\nZ=1\n", 3307).unwrap();
        assert_eq!((done.line, done.before), (3, None));
        assert_eq!(done.after, "DB_PORT=3307");
    }

    #[test]
    fn what_is_replaced_is_exactly_what_the_reader_reads() {
        // For lines of every shape, the region this module replaces holds the value `read_keys` returns.
        for line in [
            "DB_PORT=3306",
            "DB_PORT = 3306",
            "export DB_PORT=3306",
            "export\tDB_PORT=3306  # c",
            "DB_PORT=\"3306\"",
            "DB_PORT='3306' # c",
            "DB_PORT=\"unclosed",
            "DB_PORT=#x",
            "  DB_PORT=3306  ",
            "DB_PORT=a#b",
            "DB_PORT=",
        ] {
            let found = assignment(line).unwrap_or_else(|| panic!("{line:?}"));
            assert_eq!(found.key, "DB_PORT", "{line:?}");
            let read = read_keys(line, &["DB_PORT"])
                .remove("DB_PORT")
                .unwrap_or_default();
            assert_eq!(&line[found.value], read, "{line:?}");
        }
        assert!(assignment("# DB_PORT=1").is_none());
        assert!(assignment("just words").is_none());
        assert!(assignment("").is_none());
    }

    // --- the file -------------------------------------------------------------------------

    fn project(text: &str) -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(ENV_FILE), text).unwrap();
        tmp
    }

    #[test]
    fn the_preview_names_the_line_and_the_backup_and_writes_nothing() {
        let tmp = project("DB_HOST=db\nDB_PORT=3306\nDB_PASSWORD=secret-value\n");
        let planned = plan_port(tmp.path(), 3307).unwrap().unwrap();
        assert_eq!(planned.port, 3307);
        assert_eq!(planned.line, 2);
        assert_eq!(planned.before.as_deref(), Some("DB_PORT=3306"));
        assert_eq!(planned.after, "DB_PORT=3307");
        assert_eq!(planned.backup, BACKUP_FILE);
        // Nothing was written, and no other line of the file is in the preview.
        assert_eq!(
            fs::read_to_string(tmp.path().join(ENV_FILE)).unwrap(),
            "DB_HOST=db\nDB_PORT=3306\nDB_PASSWORD=secret-value\n"
        );
        assert!(!tmp.path().join(BACKUP_FILE).exists());
        assert!(!format!("{planned:?}").contains("secret-value"));
    }

    #[test]
    fn there_is_nothing_to_preview_without_a_dotenv_file_or_when_the_port_is_right() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(".env.example"), "DB_PORT=3306\n").unwrap();
        assert_eq!(plan_port(tmp.path(), 3307).unwrap(), None);
        let tmp = project("DB_PORT=3307\n");
        assert_eq!(plan_port(tmp.path(), 3307).unwrap(), None);
    }

    #[test]
    fn applying_writes_the_backup_first_then_changes_only_that_line() {
        let original = "APP_KEY=base64:abc\r\nDB_PORT=3306 # mysql\r\nDB_PASSWORD=\"p w\"\r\n";
        let tmp = project(original);
        let planned = plan_port(tmp.path(), 3307).unwrap().unwrap();

        let done = apply_port(tmp.path(), &planned).unwrap();

        assert_eq!(done.backup, BACKUP_FILE);
        assert_eq!(
            fs::read_to_string(tmp.path().join(BACKUP_FILE)).unwrap(),
            original
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join(ENV_FILE)).unwrap(),
            "APP_KEY=base64:abc\r\nDB_PORT=3307 # mysql\r\nDB_PASSWORD=\"p w\"\r\n"
        );
        // Nothing else is left in the folder: no temp file.
        let mut names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [ENV_FILE, BACKUP_FILE]);
        // And it is done: the same preview is now empty.
        assert_eq!(plan_port(tmp.path(), 3307).unwrap(), None);
    }

    #[test]
    fn an_older_backup_is_never_overwritten() {
        let tmp = project("DB_PORT=1\n");
        fs::write(tmp.path().join(BACKUP_FILE), "the first backup").unwrap();
        fs::write(tmp.path().join(format!("{BACKUP_FILE}-2")), "the second").unwrap();

        let planned = plan_port(tmp.path(), 3307).unwrap().unwrap();
        assert_eq!(planned.backup, format!("{BACKUP_FILE}-3"));
        let done = apply_port(tmp.path(), &planned).unwrap();

        assert_eq!(done.backup, format!("{BACKUP_FILE}-3"));
        assert_eq!(
            fs::read_to_string(tmp.path().join(BACKUP_FILE)).unwrap(),
            "the first backup"
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join(format!("{BACKUP_FILE}-3"))).unwrap(),
            "DB_PORT=1\n"
        );
    }

    #[test]
    fn a_file_that_changed_since_the_preview_is_not_written() {
        let tmp = project("DB_PORT=3306\n");
        let planned = plan_port(tmp.path(), 3307).unwrap().unwrap();
        // The user edits the file after looking at the preview.
        fs::write(tmp.path().join(ENV_FILE), "DB_PORT=4000\n").unwrap();

        let err = apply_port(tmp.path(), &planned).unwrap_err();

        assert_eq!(err.code, ErrorCode::EnvChanged);
        assert_eq!(err.details.unwrap()["reason"], "changed");
        assert_eq!(
            fs::read_to_string(tmp.path().join(ENV_FILE)).unwrap(),
            "DB_PORT=4000\n"
        );
        assert!(
            !tmp.path().join(BACKUP_FILE).exists(),
            "no backup without a write"
        );
    }

    #[test]
    fn a_preview_the_page_made_up_is_not_applied() {
        let tmp = project("DB_PORT=3306\nA=1\n");
        let mut forged = plan_port(tmp.path(), 3307).unwrap().unwrap();
        // The page asks for another line to be written than the one lunaRun would make.
        forged.after = "A=evil".to_owned();
        assert_eq!(
            apply_port(tmp.path(), &forged).unwrap_err().code,
            ErrorCode::EnvChanged
        );
        let mut other_port = plan_port(tmp.path(), 3307).unwrap().unwrap();
        other_port.port = 9999;
        assert_eq!(
            apply_port(tmp.path(), &other_port).unwrap_err().code,
            ErrorCode::EnvChanged
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join(ENV_FILE)).unwrap(),
            "DB_PORT=3306\nA=1\n"
        );
    }

    #[test]
    fn a_file_that_cannot_be_edited_safely_is_refused_and_left_alone() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(ENV_FILE), [0x44, 0x42, 0xff, 0xfe, b'=']).unwrap();
        let err = plan_port(tmp.path(), 3307).unwrap_err();
        assert_eq!(err.code, ErrorCode::EnvChanged);
        assert_eq!(err.details.unwrap()["reason"], "notUtf8");

        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(
            tmp.path().join(ENV_FILE),
            vec![b'#'; usize::try_from(MAX_ENV_BYTES).unwrap() + 1],
        )
        .unwrap();
        assert_eq!(
            plan_port(tmp.path(), 3307).unwrap_err().details.unwrap()["reason"],
            "tooLarge"
        );

        // A folder called .env is not a file.
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(ENV_FILE)).unwrap();
        assert_eq!(
            plan_port(tmp.path(), 3307).unwrap_err().details.unwrap()["reason"],
            "notFile"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_not_followed_and_the_permissions_of_the_file_are_kept() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let tmp = tempfile::TempDir::new().unwrap();
        let real = tmp.path().join("real.env");
        fs::write(&real, "DB_PORT=1\n").unwrap();
        symlink(&real, tmp.path().join(ENV_FILE)).unwrap();
        assert_eq!(
            plan_port(tmp.path(), 3307).unwrap_err().details.unwrap()["reason"],
            "link"
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "DB_PORT=1\n");

        let tmp = project("DB_PORT=1\n");
        fs::set_permissions(tmp.path().join(ENV_FILE), fs::Permissions::from_mode(0o600)).unwrap();
        let planned = plan_port(tmp.path(), 3307).unwrap().unwrap();
        apply_port(tmp.path(), &planned).unwrap();
        for name in [ENV_FILE, BACKUP_FILE] {
            let mode = fs::metadata(tmp.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    #[test]
    fn folders_with_a_space_and_a_non_ascii_letter_work() {
        let tmp = tempfile::TempDir::new().unwrap();
        let folder = tmp.path().join("mon projet é");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join(ENV_FILE), "DB_PORT=3306\n").unwrap();
        let planned = plan_port(&folder, 3307).unwrap().unwrap();
        apply_port(&folder, &planned).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join(ENV_FILE)).unwrap(),
            "DB_PORT=3307\n"
        );
    }
}
