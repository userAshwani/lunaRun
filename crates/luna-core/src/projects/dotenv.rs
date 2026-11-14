//! A minimal reader for `.env` files that can only return the keys it was asked for (docs/PROJECT-DETECTION.md section 4).
//!
//! **Secrets never pass through here** (golden rule 7): the caller names the keys it wants (`DB_HOST`, `DB_PORT`, ...), and a
//! line whose key is not in that list is skipped **before its value is looked at**. `DB_PASSWORD`, `APP_KEY`, API tokens
//! and mail passwords are therefore never parsed, never held, and cannot end up in a log or an error.
//!
//! Handles what Laravel's own `.env` files contain: `KEY=value`, an `export ` prefix, single and double quotes (a quoted
//! value keeps its `#` and its spaces), a trailing ` # comment` after an unquoted value, blank lines and comment lines,
//! Windows line endings and a byte order mark. `${OTHER}` references are **not** expanded: the value stays as written, so
//! `DB_PORT=${MYSQL_PORT}` is simply not a number.

use std::collections::BTreeMap;

/// The values of the `wanted` keys found in `text`; the last assignment of a key wins, as in dotenv.
pub(super) fn read_keys(text: &str, wanted: &[&str]) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line
            .strip_prefix("export")
            .filter(|rest| rest.starts_with([' ', '\t']))
            .map_or(line, str::trim_start);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim_end();
        // Only now, and only for a wanted key, is the value read.
        if let Some(name) = wanted.iter().find(|w| **w == key) {
            found.insert((*name).to_owned(), unquote(value));
        }
    }
    found
}

/// The value of one assignment: quotes removed, an unquoted value cut at ` #`.
fn unquote(raw: &str) -> String {
    let raw = raw.trim_start();
    for quote in ['"', '\''] {
        if let Some(inner) = raw.strip_prefix(quote) {
            return match inner.find(quote) {
                Some(end) => inner[..end].to_owned(),
                // An unclosed quote: take the rest of the line, as dotenv does.
                None => inner.trim_end().to_owned(),
            };
        }
    }
    let end = raw
        .match_indices('#')
        .find(|(at, _)| *at == 0 || raw[..*at].ends_with([' ', '\t']))
        .map_or(raw.len(), |(at, _)| at);
    raw[..end].trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [&str; 3] = ["DB_HOST", "DB_PORT", "DB_DATABASE"];

    fn read(text: &str) -> BTreeMap<String, String> {
        read_keys(text, &KEYS)
    }

    fn value(text: &str, key: &str) -> Option<String> {
        read(text).remove(key)
    }

    #[test]
    fn plain_assignments_and_the_comment_and_blank_lines_around_them() {
        let text = "# database\n\nDB_HOST=127.0.0.1\nDB_PORT=3306\n  DB_DATABASE=shop  \n# DB_HOST=elsewhere\n";
        let found = read(text);
        assert_eq!(found["DB_HOST"], "127.0.0.1");
        assert_eq!(found["DB_PORT"], "3306");
        assert_eq!(found["DB_DATABASE"], "shop");
    }

    #[test]
    fn quotes_keep_their_content_and_an_export_prefix_is_ignored() {
        assert_eq!(
            value("DB_DATABASE=\"my shop # one\"", "DB_DATABASE").as_deref(),
            Some("my shop # one")
        );
        assert_eq!(
            value("DB_DATABASE='it\"s'", "DB_DATABASE").as_deref(),
            Some("it\"s")
        );
        assert_eq!(
            value("export DB_HOST=localhost", "DB_HOST").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            value("export\tDB_HOST=localhost", "DB_HOST").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            value("DB_DATABASE=\"unclosed", "DB_DATABASE").as_deref(),
            Some("unclosed")
        );
        assert_eq!(
            value("DB_DATABASE=\"\"", "DB_DATABASE").as_deref(),
            Some("")
        );
        assert_eq!(
            value("DB_DATABASE=\"a\" # because", "DB_DATABASE").as_deref(),
            Some("a")
        );
    }

    #[test]
    fn a_trailing_comment_is_cut_but_a_hash_inside_a_value_is_not() {
        assert_eq!(
            value("DB_PORT=3306 # default", "DB_PORT").as_deref(),
            Some("3306")
        );
        assert_eq!(
            value("DB_PORT=3306\t# default", "DB_PORT").as_deref(),
            Some("3306")
        );
        assert_eq!(
            value("DB_DATABASE=shop#1", "DB_DATABASE").as_deref(),
            Some("shop#1")
        );
        assert_eq!(
            value("DB_DATABASE=#nothing", "DB_DATABASE").as_deref(),
            Some("")
        );
    }

    #[test]
    fn windows_line_endings_a_byte_order_mark_and_spaces_around_the_equals_sign() {
        assert_eq!(
            value("DB_HOST=db\r\nDB_PORT=3307\r\n", "DB_PORT").as_deref(),
            Some("3307")
        );
        assert_eq!(
            value("\u{feff}DB_HOST=db", "DB_HOST").as_deref(),
            Some("db")
        );
        assert_eq!(value("DB_HOST = db", "DB_HOST").as_deref(), Some("db"));
    }

    #[test]
    fn the_last_assignment_wins_and_references_are_not_expanded() {
        assert_eq!(
            value("DB_PORT=1\nDB_PORT=2", "DB_PORT").as_deref(),
            Some("2")
        );
        assert_eq!(
            value("DB_PORT=${MYSQL_PORT}", "DB_PORT").as_deref(),
            Some("${MYSQL_PORT}")
        );
    }

    #[test]
    fn lines_without_an_equals_sign_and_empty_keys_are_skipped() {
        let found = read("just words\n=orphan\nDB_HOST\nDB_PORT=3306\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found["DB_PORT"], "3306");
    }

    #[test]
    fn only_the_wanted_keys_are_ever_returned_so_a_password_cannot_leak() {
        let text = "DB_PASSWORD=hunter2\nAPP_KEY=base64:abc\nMAIL_PASSWORD=\"also secret\"\nDB_HOST=db\nSTRIPE_SECRET=sk_live_x\n";
        let found = read(text);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["DB_HOST"]);
        let everything = format!("{found:?}");
        for secret in ["hunter2", "base64", "also secret", "sk_live"] {
            assert!(
                !everything.contains(secret),
                "{secret} must not be in what is returned"
            );
        }
        // A key that merely starts like a wanted one is not it.
        assert!(read("DB_HOSTNAME=x\nXDB_HOST=y\n").is_empty());
    }
}
