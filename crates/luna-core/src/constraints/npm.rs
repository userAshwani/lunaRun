//! The npm range syntax (`engines.node` of `package.json`, `.nvmrc` ranges), with node-semver's meaning.
//!
//! Written from node-semver's own rules (its range grammar and the way it expands `^`, `~`, partial versions and hyphen
//! ranges into comparators) and held to its answers by `tests/constraints_oracle.rs`. The parts that surprise people:
//!
//! - `1.2` and `1.2.x` and `1.2.*` are `>=1.2.0 <1.3.0`; a bare `1` is `>=1.0.0 <2.0.0`; `*` and the empty string are everything.
//! - Comparisons with a partial version move to the next release: `>1.2` is `>=1.3.0`, `<=1.2` is `<1.3.0`, `<1` is `<1.0.0`.
//! - `^1.2.3` is `>=1.2.3 <2.0.0`, `^0.2.3` is `>=0.2.3 <0.3.0`, `^0.0.3` is `>=0.0.3 <0.0.4`, `^0` is `<1.0.0`.
//! - `~1.2.3` is `>=1.2.3 <1.3.0`, `~1.2` the same line, `~1` is `<2.0.0`; `~>` is `~`.
//! - `1.2.3 - 2.3` is `>=1.2.3 <2.4.0` (a partial upper end is a wildcard) and needs spaces around the dash.
//! - Spaces are AND, `||` is OR, whitespace after an operator is allowed (`>= 1.2.3`), and an empty alternative is `*`.
//! - A pre-release matches only a range that names a pre-release of the same `major.minor.patch`.

use super::req::{Bound, Interval, VersionReq, release_key};
use super::{ConstraintError, Parsed};
use crate::registry::RuntimeVersion;

type Result<T> = std::result::Result<T, ConstraintError>;

#[derive(Debug, Clone)]
struct Tok {
    text: String,
    start: usize,
}

/// Read an npm range.
///
/// # Errors
/// A [`ConstraintError`] naming what is wrong and where, for what node-semver refuses (a word such as `latest`, `1.2.3.4`,
/// a comma, a lone `|`, a misplaced hyphen, an operator without a version, a number too large to hold, ...).
pub fn parse_npm(text: &str) -> Result<Parsed> {
    let mut alternatives: Vec<VersionReq> = Vec::new();
    for (start, part) in split_or(text)? {
        alternatives.push(parse_set(part, start)?);
    }
    let req = alternatives
        .into_iter()
        .reduce(|a, b| a.union(&b))
        .unwrap_or_else(VersionReq::any);
    Ok(Parsed {
        req,
        warnings: Vec::new(),
    })
}

/// Split on `||`. A single `|` is not an operator.
fn split_or(text: &str) -> Result<Vec<(usize, &str)>> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'|' {
            if bytes.get(i + 1) != Some(&b'|') {
                return Err(ConstraintError::new(
                    "a single '|': alternatives are separated by '||'",
                    i,
                ));
            }
            parts.push((from, i));
            i += 2;
            from = i;
        } else {
            i += 1;
        }
    }
    parts.push((from, text.len()));
    Ok(parts
        .into_iter()
        .map(|(a, b)| {
            let raw = &text[a..b];
            let start = a + (raw.len() - raw.trim_start().len());
            (start, raw.trim())
        })
        .collect())
}

/// One alternative: comparators joined by whitespace (AND), or `from - to`. Nothing at all is `*`.
fn parse_set(part: &str, start: usize) -> Result<VersionReq> {
    let tokens = join_operators(split_whitespace(part, start))?;
    if tokens.is_empty() {
        return Ok(VersionReq::any());
    }
    if let Some(dash) = tokens.iter().find(|t| t.text == "-") {
        if tokens.len() != 3 || tokens[1].text != "-" {
            return Err(ConstraintError::new(
                "a hyphen range is written 'from - to', with whitespace on each side and nothing else around it",
                dash.start,
            ));
        }
        return hyphen_range(&tokens[0], &tokens[2]);
    }
    let mut req = VersionReq::any();
    for token in &tokens {
        req = req.intersect(&comparator(token)?);
    }
    Ok(req)
}

fn split_whitespace(part: &str, start: usize) -> Vec<Tok> {
    let mut tokens = Vec::new();
    let mut open: Option<usize> = None;
    for (i, ch) in part.char_indices() {
        if ch.is_whitespace() {
            if let Some(from) = open.take() {
                tokens.push(Tok {
                    text: part[from..i].to_owned(),
                    start: start + from,
                });
            }
        } else if open.is_none() {
            open = Some(i);
        }
    }
    if let Some(from) = open {
        tokens.push(Tok {
            text: part[from..].to_owned(),
            start: start + from,
        });
    }
    tokens
}

/// `>= 1.2.3`, `^ 1.2.3`, `~> 1.2`: an operator by itself belongs to the version after it.
fn join_operators(tokens: Vec<Tok>) -> Result<Vec<Tok>> {
    const OPERATORS: [&str; 8] = ["<", ">", "<=", ">=", "=", "~", "~>", "^"];
    let mut joined = Vec::with_capacity(tokens.len());
    let mut iter = tokens.into_iter();
    while let Some(token) = iter.next() {
        if OPERATORS.contains(&token.text.as_str()) {
            let Some(next) = iter.next() else {
                return Err(ConstraintError::new(
                    format!("'{}' needs a version after it", token.text),
                    token.start,
                ));
            };
            joined.push(Tok {
                text: format!("{}{}", token.text, next.text),
                start: token.start,
            });
        } else {
            joined.push(token);
        }
    }
    Ok(joined)
}

/// A version as npm ranges write it: each number may be missing or `x`.
struct XVer {
    major: Option<u64>,
    minor: Option<u64>,
    patch: Option<u64>,
    pre: Option<String>,
}

impl XVer {
    fn any_major(&self) -> bool {
        self.major.is_none()
    }
    fn any_minor(&self) -> bool {
        self.any_major() || self.minor.is_none()
    }
    fn any_patch(&self) -> bool {
        self.any_minor() || self.patch.is_none()
    }
}

/// `1`, `1.2`, `1.2.3`, `1.x`, `*`, `1.2.3-beta.1`, `1.2.3+build` (the build is ignored). Leading `v` and `=` are skipped.
fn parse_xver(text: &str, position: usize) -> Result<XVer> {
    let bad =
        |what: &str| ConstraintError::new(format!("'{text}' is not a version: {what}"), position);
    let trimmed = text.trim_start_matches(['v', '=']);
    let without_build = match trimmed.split_once('+') {
        Some((_, build)) if build.is_empty() || !build.split('.').all(valid_identifier_chars) => {
            return Err(bad("the build metadata after '+' is not valid"));
        }
        Some((before, _)) => before,
        None => trimmed,
    };
    let (core, pre) = match without_build.split_once('-') {
        Some((core, tag)) => (core, Some(tag)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(bad("expected one to three numbers separated by dots"));
    }
    let mut numbers: [Option<u64>; 3] = [None; 3];
    let mut wildcard = false;
    for (slot, part) in numbers.iter_mut().zip(&parts) {
        if matches!(*part, "x" | "X" | "*") {
            wildcard = true;
        } else if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad("expected numbers, 'x' or '*' separated by dots"));
        } else if part.len() > 1 && part.starts_with('0') {
            return Err(bad("a number with a leading zero"));
        } else if wildcard {
            return Err(bad("a number after a wildcard"));
        } else {
            *slot = Some(
                part.parse::<u64>()
                    .map_err(|_| bad("a number that is too large"))?,
            );
        }
    }
    let pre = match pre {
        None => None,
        Some(tag) => {
            if parts.len() != 3 {
                return Err(bad("a pre-release needs a full major.minor.patch"));
            }
            let valid = !tag.is_empty()
                && tag.split('.').all(|id| {
                    !id.is_empty()
                        && valid_identifier_chars(id)
                        && !(id.len() > 1
                            && id.starts_with('0')
                            && id.bytes().all(|b| b.is_ascii_digit()))
                });
            if !valid {
                return Err(bad("the pre-release tag after '-' is not valid"));
            }
            // node-semver accepts `1.2.x-beta` and reads it as `1.2.x`: the tag does not count next to a wildcard.
            (!wildcard).then(|| tag.to_owned())
        }
    };
    Ok(XVer {
        major: numbers[0],
        minor: numbers[1],
        patch: numbers[2],
        pre,
    })
}

fn valid_identifier_chars(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn version_from(numbers: [u64; 3], pre: Option<&str>, position: usize) -> Result<RuntimeVersion> {
    RuntimeVersion::from_numbers(&numbers, pre)
        .ok_or_else(|| ConstraintError::new("this is not a version lunaRun can compare", position))
}

fn plus_one(n: u64, position: usize) -> Result<u64> {
    n.checked_add(1)
        .ok_or_else(|| ConstraintError::new("a number that is too large", position))
}

/// Below every pre-release of `numbers` too: `<2.0.0-0`.
fn end_before(numbers: [u64; 3], position: usize) -> Result<Bound> {
    Ok(Bound::Excluded(version_from(numbers, Some("0"), position)?))
}

fn range(lower: Bound, upper: Bound, pre_of: Option<[u64; 3]>) -> VersionReq {
    VersionReq::from_parts(
        vec![Interval::new(lower, upper)],
        pre_of.into_iter().collect(),
    )
}

/// One comparator: an operator and a possibly partial version.
fn comparator(token: &Tok) -> Result<VersionReq> {
    let text = token.text.as_str();
    let (op, rest) = split_operator(text);
    let position = token.start + (text.len() - rest.len());
    if rest.trim_start_matches(['v', '=']).is_empty() {
        return Err(ConstraintError::new(
            if op.is_empty() {
                "an empty comparator".to_owned()
            } else {
                format!("'{op}' needs a version after it")
            },
            token.start,
        ));
    }
    let ver = parse_xver(rest, position)?;
    // A whole version is passed to node-semver's comparator grammar as written, which allows one `v` and nothing else
    // between the operator and the numbers. `^`, `~` and the partial versions are rewritten first, which is why
    // `^v=1.2.3` and `=1.x` are accepted and `v=1.2.3` is not.
    let prefix = &rest[..rest.len() - rest.trim_start_matches(['v', '=']).len()];
    if !matches!(op, "^" | "~" | "~>") && !ver.any_patch() && !matches!(prefix, "" | "v") {
        return Err(ConstraintError::new(
            format!("'{prefix}' in front of the version: only a single 'v' may be there"),
            position,
        ));
    }
    match op {
        "^" => caret(&ver, position),
        "~" | "~>" => tilde(&ver, position),
        _ => x_range(op, &ver, position),
    }
}

fn split_operator(text: &str) -> (&str, &str) {
    for op in ["~>", "<=", ">=", "~", "^", "<", ">", "="] {
        if let Some(rest) = text.strip_prefix(op) {
            return (op, rest);
        }
    }
    ("", text)
}

/// `^1.2.3`: nothing left of the first non-zero number may change.
fn caret(ver: &XVer, position: usize) -> Result<VersionReq> {
    let Some(major) = ver.major else {
        return Ok(VersionReq::any());
    };
    let Some(minor) = ver.minor else {
        return Ok(range(
            Bound::Included(version_from([major, 0, 0], None, position)?),
            end_before([plus_one(major, position)?, 0, 0], position)?,
            None,
        ));
    };
    let Some(patch) = ver.patch else {
        let end = if major == 0 {
            [0, plus_one(minor, position)?, 0]
        } else {
            [plus_one(major, position)?, 0, 0]
        };
        return Ok(range(
            Bound::Included(version_from([major, minor, 0], None, position)?),
            end_before(end, position)?,
            None,
        ));
    };
    let end = if major != 0 {
        [plus_one(major, position)?, 0, 0]
    } else if minor != 0 {
        [0, plus_one(minor, position)?, 0]
    } else {
        [0, 0, plus_one(patch, position)?]
    };
    let low = version_from([major, minor, patch], ver.pre.as_deref(), position)?;
    let pre_of = ver.pre.as_ref().map(|_| release_key(&low));
    Ok(range(
        Bound::Included(low),
        end_before(end, position)?,
        pre_of,
    ))
}

/// `~1.2.3`: the minor may not change; with no minor, the major may not.
fn tilde(ver: &XVer, position: usize) -> Result<VersionReq> {
    let Some(major) = ver.major else {
        return Ok(VersionReq::any());
    };
    let Some(minor) = ver.minor else {
        return Ok(range(
            Bound::Included(version_from([major, 0, 0], None, position)?),
            end_before([plus_one(major, position)?, 0, 0], position)?,
            None,
        ));
    };
    let end = end_before([major, plus_one(minor, position)?, 0], position)?;
    let Some(patch) = ver.patch else {
        return Ok(range(
            Bound::Included(version_from([major, minor, 0], None, position)?),
            end,
            None,
        ));
    };
    let low = version_from([major, minor, patch], ver.pre.as_deref(), position)?;
    let pre_of = ver.pre.as_ref().map(|_| release_key(&low));
    Ok(range(Bound::Included(low), end, pre_of))
}

/// A comparator with `<`, `>`, `<=`, `>=`, `=` or nothing in front, where the version may be partial or `x`.
fn x_range(op: &str, ver: &XVer, position: usize) -> Result<VersionReq> {
    let any = ver.any_patch();
    // `=1.x` is `1.x`.
    let op = if op == "=" && any { "" } else { op };
    if ver.any_major() {
        // `>*` and `<*` hold nothing; every other use of `*` holds everything.
        return Ok(if matches!(op, ">" | "<") {
            VersionReq::none()
        } else {
            VersionReq::any()
        });
    }
    let major = ver.major.unwrap_or(0);
    let minor = if ver.any_minor() {
        0
    } else {
        ver.minor.unwrap_or(0)
    };
    let patch = if any { 0 } else { ver.patch.unwrap_or(0) };
    if !op.is_empty() && any {
        let next_release = || -> Result<[u64; 3]> {
            Ok(if ver.any_minor() {
                [plus_one(major, position)?, 0, 0]
            } else {
                [major, plus_one(minor, position)?, 0]
            })
        };
        return Ok(match op {
            ">" => range(
                Bound::Included(version_from(next_release()?, None, position)?),
                Bound::Unbounded,
                None,
            ),
            "<=" => range(
                Bound::Unbounded,
                end_before(next_release()?, position)?,
                None,
            ),
            "<" => range(
                Bound::Unbounded,
                end_before([major, minor, patch], position)?,
                None,
            ),
            _ => range(
                Bound::Included(version_from([major, minor, patch], None, position)?),
                Bound::Unbounded,
                None,
            ),
        });
    }
    if ver.any_minor() {
        return Ok(range(
            Bound::Included(version_from([major, 0, 0], None, position)?),
            end_before([plus_one(major, position)?, 0, 0], position)?,
            None,
        ));
    }
    if ver.any_patch() {
        return Ok(range(
            Bound::Included(version_from([major, minor, 0], None, position)?),
            end_before([major, plus_one(minor, position)?, 0], position)?,
            None,
        ));
    }
    // A whole version.
    let v = version_from([major, minor, patch], ver.pre.as_deref(), position)?;
    let pre_of = ver.pre.as_ref().map(|_| release_key(&v));
    Ok(match op {
        ">" => range(Bound::Excluded(v), Bound::Unbounded, pre_of),
        ">=" => range(Bound::Included(v), Bound::Unbounded, pre_of),
        "<" => range(Bound::Unbounded, Bound::Excluded(v), pre_of),
        "<=" => range(Bound::Unbounded, Bound::Included(v), pre_of),
        _ => range(Bound::Included(v.clone()), Bound::Included(v), pre_of),
    })
}

/// `from - to`: a partial `from` is filled with zeros, a partial `to` is a wildcard.
fn hyphen_range(from: &Tok, to: &Tok) -> Result<VersionReq> {
    let low_ver = parse_xver(&from.text, from.start)?;
    let high_ver = parse_xver(&to.text, to.start)?;
    let mut keys = Vec::new();

    let lower = if low_ver.any_major() {
        Bound::Unbounded
    } else {
        let major = low_ver.major.unwrap_or(0);
        let minor = if low_ver.any_minor() {
            0
        } else {
            low_ver.minor.unwrap_or(0)
        };
        if low_ver.any_patch() {
            Bound::Included(version_from([major, minor, 0], None, from.start)?)
        } else {
            let v = version_from(
                [major, minor, low_ver.patch.unwrap_or(0)],
                low_ver.pre.as_deref(),
                from.start,
            )?;
            if low_ver.pre.is_some() {
                keys.push(release_key(&v));
            }
            Bound::Included(v)
        }
    };
    let upper = if high_ver.any_major() {
        Bound::Unbounded
    } else {
        let major = high_ver.major.unwrap_or(0);
        if high_ver.any_minor() {
            end_before([plus_one(major, to.start)?, 0, 0], to.start)?
        } else {
            let minor = high_ver.minor.unwrap_or(0);
            if high_ver.any_patch() {
                end_before([major, plus_one(minor, to.start)?, 0], to.start)?
            } else {
                let v = version_from(
                    [major, minor, high_ver.patch.unwrap_or(0)],
                    high_ver.pre.as_deref(),
                    to.start,
                )?;
                if high_ver.pre.is_some() {
                    keys.push(release_key(&v));
                }
                Bound::Included(v)
            }
        }
    };
    Ok(VersionReq::from_parts(
        vec![Interval::new(lower, upper)],
        keys,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> RuntimeVersion {
        RuntimeVersion::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    fn shown(range: &str) -> String {
        parse_npm(range)
            .unwrap_or_else(|e| panic!("{range:?} should parse: {e}"))
            .req
            .to_string()
    }

    fn error(range: &str) -> ConstraintError {
        match parse_npm(range) {
            Ok(parsed) => panic!("{range:?} should be refused, got {}", parsed.req),
            Err(e) => e,
        }
    }

    #[test]
    fn caret_keeps_everything_left_of_the_first_non_zero_number() {
        for (text, expected) in [
            ("^1.2.3", ">=1.2.3 <2.0.0"),
            ("^1.2", ">=1.2.0 <2.0.0"),
            ("^1", ">=1.0.0 <2.0.0"),
            ("^0.2.3", ">=0.2.3 <0.3.0"),
            ("^0.2", ">=0.2.0 <0.3.0"),
            ("^0.0.3", ">=0.0.3 <0.0.4"),
            ("^0.0", ">=0.0.0 <0.1.0"),
            ("^0.0.x", ">=0.0.0 <0.1.0"),
            ("^0", ">=0.0.0 <1.0.0"),
            ("^0.x", ">=0.0.0 <1.0.0"),
            ("^1.x", ">=1.0.0 <2.0.0"),
            ("^1.2.x", ">=1.2.0 <2.0.0"),
            ("^*", "*"),
            ("^x", "*"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
    }

    #[test]
    fn tilde_keeps_the_minor_unless_there_is_none() {
        for (text, expected) in [
            ("~1.2.3", ">=1.2.3 <1.3.0"),
            ("~1.2", ">=1.2.0 <1.3.0"),
            ("~1", ">=1.0.0 <2.0.0"),
            ("~0", ">=0.0.0 <1.0.0"),
            ("~0.0.1", ">=0.0.1 <0.1.0"),
            ("~>1.2.3", ">=1.2.3 <1.3.0"),
            ("~> 1.2.3", ">=1.2.3 <1.3.0"),
            ("~1.x", ">=1.0.0 <2.0.0"),
            ("~*", "*"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
    }

    #[test]
    fn partial_versions_and_x_ranges() {
        for (text, expected) in [
            ("1", ">=1.0.0 <2.0.0"),
            ("1.2", ">=1.2.0 <1.3.0"),
            ("1.2.x", ">=1.2.0 <1.3.0"),
            ("1.2.*", ">=1.2.0 <1.3.0"),
            ("1.x", ">=1.0.0 <2.0.0"),
            ("1.X.x", ">=1.0.0 <2.0.0"),
            ("1.2.x-beta", ">=1.2.0 <1.3.0"),
            ("x.x.x-beta", "*"),
            ("v1.x", ">=1.0.0 <2.0.0"),
            ("=v1.2.3", "=1.2.3"),
            ("^v=1.2.3", ">=1.2.3 <2.0.0"),
            ("v=1.x", ">=1.0.0 <2.0.0"),
            ("=1.2", ">=1.2.0 <1.3.0"),
            ("*", "*"),
            ("x", "*"),
            ("", "*"),
            ("   ", "*"),
            ("1.2.3", "=1.2.3"),
            ("=1.2.3", "=1.2.3"),
            ("v1.2.3", "=1.2.3"),
            ("=v1.2.3", "=1.2.3"),
            ("1.2.3+build.5", "=1.2.3"),
        ] {
            assert_eq!(shown(text), expected, "{text:?}");
        }
    }

    #[test]
    fn comparisons_with_a_partial_version_move_to_the_next_release() {
        for (text, expected) in [
            (">1.2.3", ">1.2.3"),
            (">=1.2.3", ">=1.2.3"),
            ("<1.2.3", "<1.2.3"),
            ("<=1.2.3", "<=1.2.3"),
            (">1.2", ">=1.3.0"),
            (">1", ">=2.0.0"),
            (">=1.2", ">=1.2.0"),
            (">=1", ">=1.0.0"),
            ("<1.2", "<1.2.0"),
            ("<1", "<1.0.0"),
            ("<=1.2", "<1.3.0"),
            ("<=1", "<2.0.0"),
            (">=*", "*"),
            ("<=*", "*"),
            (">=18", ">=18.0.0"),
            ("<19", "<19.0.0"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
        assert!(parse_npm(">*").unwrap().req.is_empty());
        assert!(parse_npm("<*").unwrap().req.is_empty());
        assert!(parse_npm(">1.x").unwrap().req.matches(&v("2.0.0")));
        assert!(!parse_npm(">1.x").unwrap().req.matches(&v("1.9.9")));
    }

    #[test]
    fn hyphen_ranges_fill_a_partial_start_and_wildcard_a_partial_end() {
        for (text, expected) in [
            ("1.2.3 - 2.3.4", ">=1.2.3 <=2.3.4"),
            ("1.2 - 2.3.4", ">=1.2.0 <=2.3.4"),
            ("1.2.3 - 2.3", ">=1.2.3 <2.4.0"),
            ("1.2.3 - 2", ">=1.2.3 <3.0.0"),
            ("1 - 2", ">=1.0.0 <3.0.0"),
            ("1.x - 2.x", ">=1.0.0 <3.0.0"),
            ("* - 2", "<3.0.0"),
            ("1 - *", ">=1.0.0"),
            ("0 - 0", ">=0.0.0 <1.0.0"),
            ("1.2.3   -   2.3.4", ">=1.2.3 <=2.3.4"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
        assert!(
            parse_npm("2.3.4 - 1.2.3").unwrap().req.is_empty(),
            "backwards: nothing"
        );
        assert_eq!(
            shown("1.2.3 - 2.3.4 || 5"),
            ">=1.2.3 <=2.3.4 || >=5.0.0 <6.0.0"
        );
    }

    #[test]
    fn space_is_and_double_pipe_is_or_and_an_empty_alternative_is_everything() {
        assert_eq!(shown(">=1.2.3 <2.0.0"), ">=1.2.3 <2.0.0");
        assert_eq!(shown(">=1.2.3\t<2.0.0"), ">=1.2.3 <2.0.0");
        assert_eq!(shown(">= 1.2.3  <  2.0.0"), ">=1.2.3 <2.0.0");
        assert_eq!(shown("^ 1.2.3"), ">=1.2.3 <2.0.0");
        assert_eq!(shown("1 ||"), "*");
        assert_eq!(shown("|| 1"), "*");
        assert_eq!(shown("||"), "*");
        assert_eq!(
            shown("^16.14.0 || >=18.0.0"),
            ">=16.14.0 <17.0.0 || >=18.0.0"
        );
        assert_eq!(
            shown("^16 || ^17"),
            ">=16.0.0 <18.0.0",
            "touching alternatives become one"
        );
        assert_eq!(
            shown("1.x || >=2.5.0 || 5.0.0 - 7.2.3"),
            ">=1.0.0 <2.0.0 || >=2.5.0"
        );
        assert_eq!(shown("<1.0.0 >2.0.0"), "no version");
        assert_eq!(shown("1.2.3||2.0.0"), "=1.2.3 || =2.0.0");
        assert_eq!(shown("  >=18  "), ">=18.0.0");
    }

    #[test]
    fn a_prerelease_matches_only_a_range_that_names_one_of_the_same_release() {
        let stable = parse_npm("^1.2.3").unwrap().req;
        assert!(
            stable.matches(&v("1.9.9")) && !stable.matches(&v("1.2.4-alpha")),
            "nobody asked for pre-releases"
        );
        let named = parse_npm("^1.2.3-alpha.1").unwrap().req;
        assert!(named.allows_prereleases());
        assert!(
            named.matches(&v("1.2.3-alpha.1"))
                && named.matches(&v("1.2.3-beta"))
                && named.matches(&v("1.2.3"))
        );
        assert!(
            !named.matches(&v("1.2.3-alpha")),
            "older than the one it starts at"
        );
        assert!(
            !named.matches(&v("1.2.4-alpha")),
            "a pre-release of another release"
        );
        assert!(!named.matches(&v("2.0.0-rc.1")));
        // `<2.0.0` stays below every pre-release of 2.0.0.
        assert!(!parse_npm("1.x").unwrap().req.matches(&v("2.0.0-rc.1")));
        assert!(!parse_npm("<2.0.0-0").unwrap().req.matches(&v("2.0.0-0")));
        assert!(
            parse_npm("1.2.3-beta")
                .unwrap()
                .req
                .matches(&v("1.2.3-beta"))
        );
        assert!(!parse_npm("1.2.3-beta").unwrap().req.matches(&v("1.2.3")));
    }

    #[test]
    fn the_real_node_engines_fields() {
        let accepts = |text: &str, yes: &[&str], no: &[&str]| {
            let req = parse_npm(text)
                .unwrap_or_else(|e| panic!("{text}: {e}"))
                .req;
            for version in yes {
                assert!(req.matches(&v(version)), "{text} should accept {version}");
            }
            for version in no {
                assert!(!req.matches(&v(version)), "{text} should refuse {version}");
            }
        };
        accepts(
            ">=18",
            &["18.0.0", "20.11.1", "24.22.2"],
            &["16.20.2", "17.9.1"],
        );
        accepts(
            "^16.14.0 || >=18.0.0",
            &["16.14.0", "16.20.2", "18.0.0", "22.1.0"],
            &["16.13.0", "17.0.0"],
        );
        accepts(
            "^18.17.0 || >=20.5.0",
            &["18.17.0", "18.20.4", "20.5.0", "22.1.0"],
            &["18.16.0", "19.0.0", "20.4.0"],
        );
        accepts(
            "^12.20.0 || ^14.13.1 || >=16.0.0",
            &["12.22.0", "14.13.1", "16.0.0"],
            &["13.0.0", "14.13.0"],
        );
        accepts(
            ">=6.9.0 <7 || >=8",
            &["6.9.0", "6.17.1", "8.0.0"],
            &["7.0.0", "6.8.0"],
        );
        accepts("20.x", &["20.0.0", "20.11.1"], &["19.9.9", "21.0.0"]);
        accepts(
            "18 || 20 || 22",
            &["18.1.0", "20.5.0", "22.0.0"],
            &["19.0.0", "21.0.0"],
        );
        accepts(">=10 <15", &["10.0.0", "14.21.3"], &["9.11.2", "15.0.0"]);
    }

    #[test]
    fn what_node_semver_refuses_is_refused_with_a_position() {
        for text in [
            "abc",
            "latest",
            "^",
            "~",
            ">=",
            "1..2",
            "1.2.",
            "^^1.2",
            "1.2.3.4",
            "^1.2.3 abc",
            "%",
            "1,2",
            ">=1.2.3,<2.0.0",
            ">=>1",
            "=1.2.3 =",
            "1 | 2",
            "|",
            "1.2.3 -",
            "- 1.2.3",
            "1.2.3 - 2.0.0 - 3.0.0",
            ">=1.0.0 - 2.0.0",
            "01.2.3",
            "1.02.3",
            "1.2.3-",
            "1.2.3-.1",
            "1.2.3-beta..1",
            "1.2.3-01",
            "1.x.3",
            "1.*.3",
            "v=1.2.3",
            "=v=1.2.3",
            ">==1.2.3",
            "V1.2.3",
            "1.x-beta",
            "1.2.3+",
            "99999999999999999999999.1",
            "^18446744073709551615",
            "~18446744073709551615",
            "v",
            "=",
            "1.2.3 ||| 1",
        ] {
            let e = error(text);
            assert!(!e.message.is_empty(), "{text:?}");
            assert!(
                e.position <= text.len(),
                "{text:?}: position {} is past the end",
                e.position
            );
        }
    }

    #[test]
    fn the_position_points_at_the_part_that_is_wrong() {
        assert_eq!(error("^16 || abc").position, 7);
        assert_eq!(error(">=1 <x.y").position, 5);
        assert_eq!(error("1 | 2").position, 2);
        assert_eq!(error("1.2.3 - 2.0.0 - 3.0.0").position, 6);
        assert!(error("abc").to_string().contains("at position 0"));
    }
}
