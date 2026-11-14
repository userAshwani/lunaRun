//! The Composer constraint syntax (`require.php` of `composer.json`, `packages[].require.php` of `composer.lock`).
//!
//! Follows Composer's own parser (`Composer\Semver\VersionParser::parseConstraints`) and is held to its answers by
//! `tests/constraints_oracle.rs`. The points that surprise people, all from Composer:
//!
//! - A **bare version is exact**, even a partial one: `8.1` is `==8.1.0`, not "8.1.x". `8.1.*` is the wildcard.
//! - `<=8.1` is `<=8.1.0`: it does not include `8.1.5`. `>8.1` is `>8.1.0`. `>=8.1` and `<8.1` are what they look like.
//! - `~8.1` is `>=8.1.0 <9.0.0` (one or two numbers move the major), `~8.1.2` is `>=8.1.2 <8.2.0`.
//! - `^0.3` is `>=0.3.0 <0.4.0`, `^0.0.3` is `>=0.0.3 <0.0.4`: the first non-zero number is the one that must not change.
//! - Constraints are separated by a **space or a comma** (AND) and by `||` **or a single `|`** (OR). `>= 8.1` with a space
//!   after the operator is `>=8.1`; `^ 8.1` and a tab between constraints are errors.
//! - `8.1 - 8.3` is `>=8.1.0 <8.4.0`: a partial upper end is a wildcard (`8.1 - 8.3.5` is `<=8.3.5`).
//!
//! Where lunaRun deliberately differs from Composer is listed in D-034: a development branch (`dev-main`) cannot be
//! compared with a PHP version, so it is left out with a warning instead of matching nothing; and a few sloppy spellings
//! Composer lets through (`8.1.` with a trailing dot) are errors here.

use super::req::{Bound, Interval, VersionReq, release_key};
use super::{ConstraintError, ConstraintWarning, Parsed};
use crate::registry::RuntimeVersion;

type Result<T> = std::result::Result<T, ConstraintError>;

/// A piece of text and where it starts in the whole constraint.
#[derive(Debug, Clone)]
struct Tok {
    text: String,
    start: usize,
}

/// What one token of a group makes: a requirement, or a development branch that cannot be compared.
enum Piece {
    Req(VersionReq),
    Branch,
}

/// What Composer trims from the ends of a constraint.
const BLANKS: [char; 4] = [' ', '\t', '\n', '\r'];

/// The stability words after `@` (`^8.1@dev`).
const STABILITIES: [&str; 5] = ["dev", "alpha", "beta", "rc", "stable"];

/// The words a version may end with after a dash (`1.0.0-beta2`, `2.0-RC1`, `1.0-dev`).
const SUFFIX_WORDS: [&str; 10] = [
    "dev", "alpha", "a", "beta", "b", "rc", "patch", "pl", "p", "stable",
];

/// Read a Composer constraint.
///
/// # Errors
/// A [`ConstraintError`] naming what is wrong and where, for text Composer itself would refuse (an empty constraint, a
/// misplaced hyphen or comma, an operator without a version, `^8.*`, a version with five numbers, ...).
pub fn parse_composer(text: &str) -> Result<Parsed> {
    let mut warnings = Vec::new();
    let mut alternatives: Vec<VersionReq> = Vec::new();
    for (start, part) in split_or(text)? {
        let (req, mut found) = parse_group(part, start)?;
        warnings.append(&mut found);
        // A group made only of development branches does not restrict anything, but it must not widen the others.
        alternatives.extend(req);
    }
    let req = alternatives
        .into_iter()
        .reduce(|a, b| a.union(&b))
        .unwrap_or_else(VersionReq::any);
    Ok(Parsed { req, warnings })
}

/// Split on `||` and on a single `|`, trimming each part. Returns each part with where it starts.
fn split_or(text: &str) -> Result<Vec<(usize, &str)>> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'|' {
            let run = bytes[i..].iter().take_while(|b| **b == b'|').count();
            if run > 2 {
                return Err(ConstraintError::new(
                    "unexpected '|': alternatives are separated by '||'",
                    i,
                ));
            }
            parts.push((from, i));
            i += run;
            from = i;
        } else {
            i += 1;
        }
    }
    parts.push((from, text.len()));
    parts
        .into_iter()
        .map(|(a, b)| {
            let raw = &text[a..b];
            let trimmed = raw.trim_matches(BLANKS);
            let start = a + (raw.len() - raw.trim_start_matches(BLANKS).len());
            if trimmed.is_empty() {
                Err(ConstraintError::new(
                    "an empty constraint (nothing before or after '||')",
                    a,
                ))
            } else {
                Ok((start, trimmed))
            }
        })
        .collect()
}

/// One alternative: constraints joined by a space or a comma (AND), or `from - to`. `None` when it is only development
/// branches.
fn parse_group(part: &str, start: usize) -> Result<(Option<VersionReq>, Vec<ConstraintWarning>)> {
    let (tokens, used_comma) = split_and(part, start)?;
    let tokens = join_operators(drop_alias(tokens))?;

    if let Some(dash) = tokens.iter().find(|t| t.text == "-") {
        let well_formed = !used_comma && tokens.len() == 3 && tokens[1].text == "-";
        if !well_formed {
            return Err(ConstraintError::new(
                "a hyphen range is written 'from - to', with a space on each side and nothing else around it",
                dash.start,
            ));
        }
        let (from, to) = (&tokens[0], &tokens[2]);
        return Ok((Some(hyphen_range(from, to)?), Vec::new()));
    }

    let mut req: Option<VersionReq> = None;
    let mut warnings = Vec::new();
    for token in &tokens {
        match parse_token(token)? {
            Piece::Req(next) => {
                req = Some(match req {
                    Some(so_far) => so_far.intersect(&next),
                    None => next,
                });
            }
            Piece::Branch => warnings.push(ConstraintWarning::DevBranch(token.text.clone())),
        }
    }
    Ok((req, warnings))
}

/// Split a group on spaces and commas. A comma may sit between two constraints, with spaces around it, once.
fn split_and(part: &str, start: usize) -> Result<(Vec<Tok>, bool)> {
    let mut tokens: Vec<Tok> = Vec::new();
    let mut open: Option<usize> = None;
    let mut comma_pending = false;
    let mut used_comma = false;
    let close = |tokens: &mut Vec<Tok>, open: &mut Option<usize>, end: usize| {
        if let Some(from) = open.take() {
            tokens.push(Tok {
                text: part[from..end].to_owned(),
                start: start + from,
            });
        }
    };
    for (i, ch) in part.char_indices() {
        match ch {
            ' ' => close(&mut tokens, &mut open, i),
            ',' => {
                close(&mut tokens, &mut open, i);
                if tokens.is_empty() {
                    return Err(ConstraintError::new(
                        "a comma with no constraint before it",
                        start + i,
                    ));
                }
                if comma_pending {
                    return Err(ConstraintError::new("two commas in a row", start + i));
                }
                comma_pending = true;
                used_comma = true;
            }
            _ => {
                if open.is_none() {
                    open = Some(i);
                    comma_pending = false;
                }
            }
        }
    }
    close(&mut tokens, &mut open, part.len());
    if comma_pending {
        return Err(ConstraintError::new(
            "a comma with no constraint after it",
            start + part.len() - 1,
        ));
    }
    Ok((tokens, used_comma))
}

/// `dev-main as 8.1.0` is `dev-main`: the alias after `as` only names the version the branch pretends to be.
fn drop_alias(tokens: Vec<Tok>) -> Vec<Tok> {
    if tokens.len() == 3 && tokens[1].text == "as" {
        return tokens.into_iter().take(1).collect();
    }
    tokens
}

/// `>= 8.1` is `>=8.1`: an operator by itself belongs to the version after it. (`^ 8.1` and `~ 8.1` are not joined: Composer
/// refuses them.)
fn join_operators(tokens: Vec<Tok>) -> Result<Vec<Tok>> {
    const OPERATORS: [&str; 8] = [">=", "<=", ">", "<", "=", "==", "!=", "<>"];
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

/// The version part of a token, read.
struct Ver {
    /// The numbers written: 1 to 4, or 0 to 3 in front of wildcards.
    numbers: Vec<u64>,
    /// Ended in `.x`, `.X` or `.*` (or was only that).
    wildcard: bool,
    /// The tag after a dash: `beta2`, `dev`.
    pre: Option<String>,
}

fn parse_version(text: &str, position: usize) -> Result<Ver> {
    let bad =
        |what: &str| ConstraintError::new(format!("'{text}' is not a version: {what}"), position);
    let trimmed = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let (core, suffix) = match trimmed.split_once('-') {
        Some((core, suffix)) => (core, Some(suffix)),
        None => (trimmed, None),
    };
    if core.is_empty() {
        return Err(bad("no numbers"));
    }
    let mut numbers = Vec::new();
    let mut wildcards = 0;
    for part in core.split('.') {
        if matches!(part, "x" | "X" | "*") {
            wildcards += 1;
        } else if wildcards > 0 {
            return Err(bad("a number after a wildcard"));
        } else if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
            numbers.push(
                part.parse::<u64>()
                    .map_err(|_| bad("a number that is too large"))?,
            );
        } else {
            return Err(bad("expected numbers separated by dots"));
        }
    }
    let wildcard = wildcards > 0;
    let most = if wildcard { 3 } else { 4 };
    if numbers.len() > most {
        return Err(bad("too many numbers"));
    }
    if numbers.is_empty() && !wildcard {
        return Err(bad("no numbers"));
    }
    let pre = match suffix {
        None => None,
        Some(_) if wildcard => return Err(bad("a wildcard and a stability suffix")),
        Some(tag) => Some(check_suffix(tag).ok_or_else(|| {
            bad("what follows the dash is not a stability such as dev, beta or RC1")
        })?),
    };
    Ok(Ver {
        numbers,
        wildcard,
        pre,
    })
}

/// `beta2`, `RC1`, `dev`, `beta.2`: a stability word and an optional number. Returns the tag lower-cased.
fn check_suffix(tag: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let word_end = lower
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(lower.len());
    let (word, rest) = lower.split_at(word_end);
    if !SUFFIX_WORDS.contains(&word) {
        return None;
    }
    let rest = rest.strip_prefix(['.', '-']).unwrap_or(rest);
    let numeric = rest.is_empty()
        || rest
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    numeric.then_some(lower)
}

fn version_from(numbers: &[u64], pre: Option<&str>, position: usize) -> Result<RuntimeVersion> {
    RuntimeVersion::from_numbers(numbers, pre)
        .ok_or_else(|| ConstraintError::new("this is not a version lunaRun can compare", position))
}

/// Up to four numbers, at least three: `8.1` is `8.1.0`.
fn padded(numbers: &[u64]) -> Vec<u64> {
    let mut all = numbers.to_vec();
    while all.len() < 3 {
        all.push(0);
    }
    all
}

/// `numbers` with the number at `at` (0 major, 1 minor, 2 patch) raised by one and those after it zeroed.
fn bump(numbers: &[u64], at: usize, position: usize) -> Result<[u64; 3]> {
    let base = padded(numbers);
    let mut out = [base[0], base[1], base[2]];
    out[at] = out[at]
        .checked_add(1)
        .ok_or_else(|| ConstraintError::new("a number that is too large", position))?;
    for later in out.iter_mut().skip(at + 1) {
        *later = 0;
    }
    Ok(out)
}

/// The exclusive end `<9.0.0-0`: below every pre-release of 9.0.0 too.
fn end_before(numbers: [u64; 3], position: usize) -> Result<Bound> {
    Ok(Bound::Excluded(version_from(
        &numbers,
        Some("0"),
        position,
    )?))
}

fn req_of(intervals: Vec<Interval>, pre_of: Option<[u64; 3]>) -> VersionReq {
    VersionReq::from_parts(intervals, pre_of.into_iter().collect())
}

fn parse_token(token: &Tok) -> Result<Piece> {
    let mut text = token.text.as_str();
    if let Some(at) = text.rfind('@') {
        let flag = &text[at + 1..];
        if !STABILITIES.contains(&flag.to_ascii_lowercase().as_str()) {
            return Err(ConstraintError::new(
                format!("'@{flag}' is not a stability flag (dev, alpha, beta, RC or stable)"),
                token.start + at,
            ));
        }
        text = &text[..at];
        if text.is_empty() {
            // `@dev` on its own: a stability and nothing else, which Composer reads as `*`.
            return Ok(Piece::Req(VersionReq::any()));
        }
    }
    let (op, rest) = split_operator(text);
    let position = token.start + (text.len() - rest.len());
    if rest.is_empty() {
        return Err(ConstraintError::new(
            if op.is_empty() {
                "an empty constraint".to_owned()
            } else {
                format!("'{op}' needs a version after it")
            },
            token.start,
        ));
    }
    if is_branch(rest) {
        return Ok(Piece::Branch);
    }
    if matches!(rest, "*" | "x" | "X") && op.is_empty() {
        return Ok(Piece::Req(VersionReq::any()));
    }

    let ver = parse_version(rest, position)?;
    if ver.wildcard {
        if !op.is_empty() {
            return Err(ConstraintError::new(
                format!("a wildcard cannot be used with '{op}'"),
                position,
            ));
        }
        return wildcard_req(&ver.numbers, position).map(Piece::Req);
    }

    let numbers = padded(&ver.numbers);
    let pre = ver.pre.as_deref();
    let low = version_from(&numbers, pre, position)?;
    let pre_of = pre.map(|_| release_key(&low));
    let from = |bound: Bound| Interval::new(bound, Bound::Unbounded);
    let upto = |bound: Bound| Interval::new(Bound::Unbounded, bound);
    let req = match op {
        "" | "=" | "==" => req_of(
            vec![Interval::new(
                Bound::Included(low.clone()),
                Bound::Included(low),
            )],
            pre_of,
        ),
        "!=" | "<>" => req_of(
            vec![
                upto(Bound::Excluded(low.clone())),
                from(Bound::Excluded(low)),
            ],
            pre_of,
        ),
        ">" => req_of(vec![from(Bound::Excluded(low))], pre_of),
        ">=" => req_of(vec![from(Bound::Included(low))], pre_of),
        "<" => {
            // `<8.4` is below every pre-release of 8.4.0 too (Composer writes it `<8.4.0.0-dev`).
            let end = if pre.is_some() {
                Bound::Excluded(low)
            } else {
                Bound::Excluded(version_from(&numbers, Some("0"), position)?)
            };
            req_of(vec![upto(end)], pre_of)
        }
        "<=" => req_of(vec![upto(Bound::Included(low))], pre_of),
        "^" => {
            // The first number that is not zero (among those written) must not change.
            let n = &ver.numbers;
            let at = if n[0] != 0 || n.len() < 2 {
                0
            } else if n[1] != 0 || n.len() < 3 {
                1
            } else {
                2
            };
            let end = end_before(bump(&numbers, at, position)?, position)?;
            req_of(vec![Interval::new(Bound::Included(low), end)], pre_of)
        }
        "~" => {
            let at = match ver.numbers.len() {
                1 | 2 => 0,
                3 => 1,
                _ => 2,
            };
            let end = end_before(bump(&numbers, at, position)?, position)?;
            req_of(vec![Interval::new(Bound::Included(low), end)], pre_of)
        }
        _ => {
            return Err(ConstraintError::new(
                format!("'{op}' is not an operator"),
                token.start,
            ));
        }
    };
    Ok(Piece::Req(req))
}

/// `8.*` is `>=8.0.0 <9.0.0`; `8.1.*` is `>=8.1.0 <8.2.0`; `*.*` is everything.
fn wildcard_req(numbers: &[u64], position: usize) -> Result<VersionReq> {
    if numbers.is_empty() {
        return Ok(VersionReq::any());
    }
    let low = version_from(&padded(numbers), None, position)?;
    let end = end_before(bump(numbers, numbers.len() - 1, position)?, position)?;
    Ok(req_of(vec![Interval::new(Bound::Included(low), end)], None))
}

/// `from - to`: `to` with fewer than three numbers is a wildcard (`8.1 - 8.3` ends below `8.4.0`).
fn hyphen_range(from: &Tok, to: &Tok) -> Result<VersionReq> {
    let low_ver = parse_version(&from.text, from.start)?;
    let high_ver = parse_version(&to.text, to.start)?;
    for (ver, tok) in [(&low_ver, from), (&high_ver, to)] {
        if ver.wildcard {
            return Err(ConstraintError::new(
                "a wildcard cannot be an end of a hyphen range",
                tok.start,
            ));
        }
    }
    let low = version_from(
        &padded(&low_ver.numbers),
        low_ver.pre.as_deref(),
        from.start,
    )?;
    let end = if high_ver.numbers.len() < 3 && high_ver.pre.is_none() {
        end_before(
            bump(&high_ver.numbers, high_ver.numbers.len() - 1, to.start)?,
            to.start,
        )?
    } else {
        Bound::Included(version_from(
            &padded(&high_ver.numbers),
            high_ver.pre.as_deref(),
            to.start,
        )?)
    };
    let mut keys = Vec::new();
    if low_ver.pre.is_some() {
        keys.push(release_key(&low));
    }
    if let (Bound::Included(top), Some(_)) = (&end, &high_ver.pre) {
        keys.push(release_key(top));
    }
    Ok(VersionReq::from_parts(
        vec![Interval::new(Bound::Included(low), end)],
        keys,
    ))
}

/// The operator at the start of a token, and the rest. Longest first.
fn split_operator(text: &str) -> (&str, &str) {
    for op in [">=", "<=", "!=", "==", "<>", ">", "<", "=", "^", "~"] {
        if let Some(rest) = text.strip_prefix(op) {
            return (op, rest);
        }
    }
    ("", text)
}

/// `dev-main`, `dev-feature/x`, `1.0.x-dev`, `8.*-dev`: a branch, not a version number.
fn is_branch(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("dev-") {
        return true;
    }
    let Some(numbers) = lower.strip_suffix("-dev") else {
        return false;
    };
    // Numbers, then at least one wildcard: `1.0.x`, `8.*`, `8.1.2.x`.
    let parts: Vec<&str> = numbers.split('.').collect();
    let digits = parts
        .iter()
        .take_while(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        .count();
    digits > 0 && digits < parts.len() && parts[digits..].iter().all(|p| matches!(*p, "x" | "*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> RuntimeVersion {
        RuntimeVersion::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    fn shown(constraint: &str) -> String {
        parse_composer(constraint)
            .unwrap_or_else(|e| panic!("{constraint:?} should parse: {e}"))
            .req
            .to_string()
    }

    fn error(constraint: &str) -> ConstraintError {
        match parse_composer(constraint) {
            Ok(parsed) => panic!("{constraint:?} should be refused, got {}", parsed.req),
            Err(e) => e,
        }
    }

    #[test]
    fn the_rows_of_the_specification_table() {
        for (text, expected) in [
            ("8.2.1", "=8.2.1"),
            ("=8.2.1", "=8.2.1"),
            ("v8.2.1", "=8.2.1"),
            (">=8.1", ">=8.1.0"),
            (">8.1", ">8.1.0"),
            ("<8.4", "<8.4.0"),
            ("<=8.3", "<=8.3.0"),
            ("!=8.2.0", "<8.2.0 || >8.2.0"),
            (">=8.1 <8.4", ">=8.1.0 <8.4.0"),
            (">=8.1, <8.4", ">=8.1.0 <8.4.0"),
            ("^7.4 || ^8.0", ">=7.4.0 <9.0.0"),
            ("^7.4 | ^8.0", ">=7.4.0 <9.0.0"),
            ("8.1 - 8.3", ">=8.1.0 <8.4.0"),
            ("~8.1", ">=8.1.0 <9.0.0"),
            ("~8.1.2", ">=8.1.2 <8.2.0"),
            ("^8.1", ">=8.1.0 <9.0.0"),
            ("^0.3", ">=0.3.0 <0.4.0"),
            ("8.2.*", ">=8.2.0 <8.3.0"),
            ("8.*", ">=8.0.0 <9.0.0"),
            ("*", "*"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
    }

    #[test]
    fn a_bare_version_is_exact_and_the_comparisons_are_literal() {
        let eight_one = parse_composer("8.1").unwrap().req;
        assert!(
            eight_one.matches(&v("8.1.0")) && !eight_one.matches(&v("8.1.5")),
            "8.1 means 8.1.0 and nothing else"
        );
        let at_most = parse_composer("<=8.3").unwrap().req;
        assert!(
            at_most.matches(&v("8.3.0")) && !at_most.matches(&v("8.3.5")),
            "<=8.3 is <=8.3.0"
        );
        let above = parse_composer(">8.1").unwrap().req;
        assert!(!above.matches(&v("8.1.0")) && above.matches(&v("8.1.1")));
        let wildcard = parse_composer("8.3.*").unwrap().req;
        assert!(wildcard.matches(&v("8.3.5")) && !wildcard.matches(&v("8.4.0")));
    }

    #[test]
    fn caret_moves_the_first_number_that_is_not_zero() {
        for (text, expected) in [
            ("^1.2.3", ">=1.2.3 <2.0.0"),
            ("^1.2", ">=1.2.0 <2.0.0"),
            ("^1", ">=1.0.0 <2.0.0"),
            ("^0.3", ">=0.3.0 <0.4.0"),
            ("^0.3.5", ">=0.3.5 <0.4.0"),
            ("^0.0.3", ">=0.0.3 <0.0.4"),
            ("^0.0", ">=0.0.0 <0.1.0"),
            ("^0", ">=0.0.0 <1.0.0"),
            ("^0.0.0", ">=0.0.0 <0.0.1"),
            ("^1.2.3.4", ">=1.2.3.4 <2.0.0"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
    }

    #[test]
    fn tilde_moves_the_major_for_one_or_two_numbers_and_the_next_one_otherwise() {
        for (text, expected) in [
            ("~8", ">=8.0.0 <9.0.0"),
            ("~8.1", ">=8.1.0 <9.0.0"),
            ("~8.1.2", ">=8.1.2 <8.2.0"),
            ("~1.2.3.4", ">=1.2.3.4 <1.2.4"),
            ("~0.3", ">=0.3.0 <1.0.0"),
        ] {
            assert_eq!(shown(text), expected, "{text}");
        }
    }

    #[test]
    fn hyphen_ranges_treat_a_partial_upper_end_as_a_wildcard() {
        assert_eq!(shown("8.1 - 8.3"), ">=8.1.0 <8.4.0");
        assert_eq!(shown("8 - 9"), ">=8.0.0 <10.0.0");
        assert_eq!(shown("8.1 - 8.3.5"), ">=8.1.0 <=8.3.5");
        assert_eq!(shown("1.0.0 - 2.0.0"), ">=1.0.0 <=2.0.0");
        assert!(
            parse_composer("8.3 - 8.1").unwrap().req.is_empty(),
            "backwards: nothing, as in Composer"
        );
        assert_eq!(shown("8.1 - 8.3 || 7.4"), "=7.4.0 || >=8.1.0 <8.4.0");
    }

    #[test]
    fn wildcards_in_every_spelling() {
        assert_eq!(shown("x"), "*");
        assert_eq!(shown("X"), "*");
        assert_eq!(shown("8.x"), ">=8.0.0 <9.0.0");
        assert_eq!(shown("8.X"), ">=8.0.0 <9.0.0");
        assert_eq!(shown("8.1.2.*"), ">=8.1.2 <8.1.3");
        assert_eq!(shown("1.*.*"), ">=1.0.0 <2.0.0");
        assert_eq!(shown("*.*"), "*");
    }

    #[test]
    fn spaces_commas_and_pipes_in_every_way_composer_allows() {
        for text in [
            ">=8.1 <8.4",
            ">=8.1  <8.4",
            ">=8.1, <8.4",
            ">=8.1 ,<8.4",
            ">=8.1,<8.4",
            ">=8.1 , <8.4",
            ">= 8.1 < 8.4",
            "  >=8.1 <8.4  ",
        ] {
            assert_eq!(shown(text), ">=8.1.0 <8.4.0", "{text:?}");
        }
        for text in [
            "^7.4 || ^8.0",
            "^7.4||^8.0",
            "^7.4 | ^8.0",
            "^7.4|^8.0",
            "^7.4 ||^8.0",
            "^7.4|| ^8.0",
        ] {
            assert_eq!(shown(text), ">=7.4.0 <9.0.0", "{text:?}");
        }
        assert_eq!(
            shown("8,1"),
            "no version",
            "8 and 1 together: nothing, as in Composer"
        );
        assert_eq!(shown("^8.1 !=8.2.0"), ">=8.1.0 <8.2.0 || >8.2.0 <9.0.0");
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(shown(">=7.4 <8.0 || >=8.1"), ">=7.4.0 <8.0.0 || >=8.1.0");
        assert_eq!(shown("<8.0 >=7.0"), ">=7.0.0 <8.0.0");
        assert_eq!(shown(">=8.1 <8.1"), "no version");
        assert_eq!(
            shown("^7.4 ^8.0"),
            "no version",
            "no version is on both sides"
        );
        assert_eq!(shown("8.1.0 8.1.0"), "=8.1.0");
    }

    #[test]
    fn stability_flags_are_ignored_and_prereleases_need_to_be_named() {
        assert_eq!(shown("^8.1@dev"), ">=8.1.0 <9.0.0");
        assert_eq!(shown("^8.1@stable"), ">=8.1.0 <9.0.0");
        assert_eq!(shown("@dev"), "*", "a flag on its own is everything");
        assert_eq!(shown("*@dev"), "*");
        assert_eq!(shown(">=8.1@beta"), ">=8.1.0");
        assert!(
            !parse_composer("^8.1").unwrap().req.matches(&v("8.2.0-rc1")),
            "stable only"
        );
        let wants = parse_composer(">=8.2.0-RC1").unwrap().req;
        assert!(wants.allows_prereleases());
        assert!(wants.matches(&v("8.2.0-rc2")) && wants.matches(&v("8.2.0")));
        assert!(!wants.matches(&v("8.3.0-rc1")));
        // `<8.4` is below 8.4.0-rc1 as well, as in Composer.
        let below = parse_composer("<8.4 || >=8.4.0-rc1").unwrap().req;
        assert!(below.matches(&v("8.3.9")) && below.matches(&v("8.4.0-rc1")));
    }

    #[test]
    fn a_development_branch_is_left_out_with_a_warning() {
        let alone = parse_composer("dev-main").unwrap();
        assert!(alone.req.is_any(), "alone it is unconstrained");
        assert_eq!(
            alone.warnings,
            [ConstraintWarning::DevBranch("dev-main".to_owned())]
        );

        let with_others = parse_composer("^8.1 || dev-main").unwrap();
        assert_eq!(
            with_others.req.to_string(),
            ">=8.1.0 <9.0.0",
            "next to others it widens nothing"
        );
        assert_eq!(with_others.warnings.len(), 1);

        let anded = parse_composer("^8.1 dev-main").unwrap();
        assert_eq!(anded.req.to_string(), ">=8.1.0 <9.0.0");

        for text in [
            "dev-master",
            "dev-feature/x",
            "8.1.x-dev",
            "dev-main as 8.1.0",
            "1.x-dev",
            "8.*-dev",
            "8.x-dev",
            "1.2.*-dev",
            "8.1.2.x-dev",
            "dev-main@dev",
            ">=dev-main",
        ] {
            let parsed = parse_composer(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert!(parsed.req.is_any(), "{text}");
            assert_eq!(parsed.warnings.len(), 1, "{text}");
        }
        assert!(parse_composer("^8.1").unwrap().warnings.is_empty());
    }

    #[test]
    fn what_composer_refuses_is_refused_with_a_position() {
        for text in [
            "",
            "  ",
            "abc",
            "^",
            "~",
            ">=",
            "8..1",
            "^^8.1",
            "8.1 -",
            "- 8.1",
            ">=>8.1",
            "8.1.2.3.4",
            "^8.1 abc",
            "%",
            "^8.1,,<9",
            ",^8.1",
            "||",
            "|",
            "8.1 as",
            "^a.b",
            ">=8.*",
            "^8.*",
            "~8.*",
            "!=8.*",
            "<8.*",
            "8.1 -8.3",
            "8.1- 8.3",
            "8.1-8.3",
            ">=8.1\t<8.4",
            "^ 8.1",
            "~ 8.1",
            "^8.1 ||",
            "|| ^8.1",
            "^8.1 || || ^9",
            "^8.1 ,",
            "8.1.",
            "1.2.3.4.5",
            "^8.1@nope",
            "8.*.1",
            "8.1.0-foo",
            "8.1.0-8",
            "99999999999999999999999.1",
            "^8.1|||^9",
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
        assert_eq!(error("^8.1 || abc").position, 8);
        assert_eq!(error(">=8.1 <8.x4").position, 7);
        assert_eq!(error("^8.1 ||").position, 7);
        assert_eq!(error("^8.1,,<9").position, 5);
        assert_eq!(error("8.1 -8.3").position, 4);
        assert!(error("abc").to_string().contains("at position 0"));
    }

    #[test]
    fn a_number_too_large_to_hold_is_an_error_not_a_wrong_answer() {
        let e = error("^18446744073709551615");
        assert!(e.message.contains("too large"), "{e}");
        error("~18446744073709551615.1");
        error("18446744073709551615.*");
        error("1 - 18446744073709551615");
    }

    #[test]
    fn real_constraints_as_written_in_composer_json_files() {
        let accepts = |text: &str, yes: &[&str], no: &[&str]| {
            let req = parse_composer(text)
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
            "^7.3|^8.0",
            &["7.3.0", "7.4.33", "8.0.0", "8.5.12"],
            &["7.2.9", "9.0.0"],
        );
        accepts("^8.0.2", &["8.0.2", "8.4.0"], &["8.0.1", "9.0.0"]);
        accepts("^8.2", &["8.2.0", "8.5.12"], &["8.1.34", "9.0.0"]);
        accepts(">=8.2", &["8.2.0", "9.1.0"], &["8.1.99"]);
        accepts("^7.2.5|^8.0", &["7.2.5", "8.3.14"], &["7.2.4"]);
        accepts(
            "~5.6|~7.0",
            &["5.6.0", "5.9.9", "7.0.0", "7.4.33"],
            &["5.5.9", "8.0.0"],
        );
        accepts(
            "8.1.* || 8.2.* || 8.3.*",
            &["8.1.5", "8.2.0", "8.3.14"],
            &["8.0.30", "8.4.0"],
        );
        accepts(">=7.2.5 <8.4", &["7.2.5", "8.3.99"], &["7.2.4", "8.4.0"]);
        accepts(
            "^7.4.20 || ^8.0.7",
            &["7.4.20", "8.0.7"],
            &["7.4.19", "8.0.6"],
        );
    }
}
