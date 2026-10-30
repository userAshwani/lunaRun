//! Runtime versions and calendar dates, with correct ordering.
//!
//! Versions are compared number by number, never as text: `8.10` is newer than `8.9`, and `8.3.14` is
//! newer than `8.3.9`. A pre-release (`8.5.0-rc1`) is older than its release (`8.5.0`).

use std::cmp::Ordering;

/// A runtime version such as `8.3.14`, `24.22.2`, `11.4.4` or `8.5.0-rc1`: two to four numbers and an
/// optional pre-release tag.
#[derive(Debug, Clone)]
pub struct RuntimeVersion {
    text: String,
    numbers: Vec<u64>,
    /// Dot-separated pre-release identifiers after the `-`, empty for a release.
    pre: Vec<String>,
}

impl RuntimeVersion {
    pub fn parse(text: &str) -> Option<Self> {
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let numbers: Vec<u64> = core
            .split('.')
            .map(|part| {
                let valid = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
                if valid { part.parse().ok() } else { None }
            })
            .collect::<Option<_>>()?;
        if !(2..=4).contains(&numbers.len()) {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(tag) => {
                let identifiers: Vec<String> = tag.split('.').map(str::to_owned).collect();
                let valid = identifiers.iter().all(|id| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                });
                if !valid {
                    return None;
                }
                identifiers
            }
        };
        Some(Self {
            text: text.to_owned(),
            numbers,
            pre,
        })
    }

    /// The version exactly as written in the registry.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Does this version belong to `line`, a prefix of whole numbers such as `8.3` or `24`?
    /// `8.30.1` is not on line `8.3`.
    pub fn is_on_line(&self, line: &str) -> bool {
        let wanted: Option<Vec<u64>> = line.split('.').map(|part| part.parse().ok()).collect();
        match wanted {
            Some(wanted) if !wanted.is_empty() => self.numbers.starts_with(&wanted),
            _ => false,
        }
    }

    /// The first two numbers as a line name: `8.3.14` gives `8.3`.
    pub fn major_minor(&self) -> String {
        match self.numbers.as_slice() {
            [major, minor, ..] => format!("{major}.{minor}"),
            [major] => major.to_string(),
            [] => String::new(),
        }
    }
}

impl std::fmt::Display for RuntimeVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// `rc10` against `rc2`: compare the letters first, then the trailing number as a number.
fn identifier_order(a: &str, b: &str) -> Ordering {
    let numeric = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    match (numeric(a), numeric(b)) {
        (true, true) => {
            // Compare as numbers without risking overflow on long digit strings.
            let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
            a.len().cmp(&b.len()).then_with(|| a.cmp(b))
        }
        (true, false) => Ordering::Less, // numbers sort before words, as in SemVer
        (false, true) => Ordering::Greater,
        (false, false) => {
            let split = |s: &str| {
                let digits = s.bytes().rev().take_while(u8::is_ascii_digit).count();
                let (word, number) = s.split_at(s.len() - digits);
                (word.to_owned(), number.parse::<u64>().ok())
            };
            let ((word_a, number_a), (word_b, number_b)) = (split(a), split(b));
            word_a.cmp(&word_b).then(number_a.cmp(&number_b))
        }
    }
}

impl Ord for RuntimeVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        for i in 0..self.numbers.len().max(other.numbers.len()) {
            let a = self.numbers.get(i).copied().unwrap_or(0);
            let b = other.numbers.get(i).copied().unwrap_or(0);
            if a != b {
                return a.cmp(&b);
            }
        }
        // Same numbers: a release is newer than any pre-release of it.
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            (false, false) => {}
        }
        for i in 0..self.pre.len().max(other.pre.len()) {
            match (self.pre.get(i), other.pre.get(i)) {
                (None, Some(_)) => return Ordering::Less, // fewer identifiers is older
                (Some(_), None) => return Ordering::Greater,
                (Some(a), Some(b)) => {
                    let order = identifier_order(a, b);
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                (None, None) => {}
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for RuntimeVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Equality follows ordering, so `8.3` equals `8.3.0`, as it does for sorting.
impl PartialEq for RuntimeVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for RuntimeVersion {}

/// A calendar date (`2027-12-31`), enough to say whether a version line has reached end of life. Kept
/// separate from any time library so tests can pass "today" in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CalDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl CalDate {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return None;
        }
        let date = Self {
            year: year.parse().ok()?,
            month: month.parse().ok()?,
            day: day.parse().ok()?,
        };
        let leap = (date.year.is_multiple_of(4) && !date.year.is_multiple_of(100))
            || date.year.is_multiple_of(400);
        let days_in_month = match date.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        (date.day >= 1 && date.day <= days_in_month).then_some(date)
    }

    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        Self::parse(&format!("{year:04}-{month:02}-{day:02}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> RuntimeVersion {
        RuntimeVersion::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn parses_real_version_shapes_and_rejects_the_rest() {
        for ok in [
            "8.3.14",
            "24.22.2",
            "11.4.4",
            "2.10.0",
            "8.5.0-rc1",
            "9.1.0-beta.2",
            "1.2.3.4",
            "8.3",
        ] {
            assert!(RuntimeVersion::parse(ok).is_some(), "{ok}");
        }
        for bad in [
            "",
            "8",
            "v8.3.1",
            "8.3.x",
            "8..3",
            "8.3.",
            ".8.3",
            "1.2.3.4.5",
            "8.3.1-",
            "8.3.1-a..b",
            "8.3.1 ",
            "８.3.1",
            "-1.2.3",
            "1.2.-3",
            "8.3.1-é",
        ] {
            assert!(RuntimeVersion::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn numbers_compare_as_numbers_not_text() {
        assert!(v("8.10.0") > v("8.9.0"));
        assert!(v("8.3.14") > v("8.3.9"));
        assert!(v("10.0.0") > v("9.9.9"));
        assert!(v("24.22.2") > v("24.4.0"));
        assert!(v("8.3.14") < v("8.3.14.1"));
        assert!(v("11.4.4") > v("11.4"), "11.4 is 11.4.0");
    }

    #[test]
    fn a_release_is_newer_than_its_prereleases_and_tags_order_sensibly() {
        let ascending = [
            "8.5.0-alpha",
            "8.5.0-beta1",
            "8.5.0-beta2",
            "8.5.0-beta10",
            "8.5.0-rc1",
            "8.5.0-rc2",
            "8.5.0",
            "8.5.1-rc1",
            "8.5.1",
        ];
        for pair in ascending.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
            assert!(v(pair[1]) > v(pair[0]));
        }
        assert!(v("8.5.0-rc1").is_prerelease());
        assert!(!v("8.5.0").is_prerelease());
    }

    #[test]
    fn equal_versions_are_equal_and_sorting_puts_newest_first() {
        assert_eq!(v("8.3"), v("8.3.0"));
        assert_ne!(v("8.3.0"), v("8.3.1"));
        let mut versions = [
            v("8.9.0"),
            v("8.10.1"),
            v("8.3.14"),
            v("8.10.0"),
            v("7.4.33"),
            v("8.4.0-rc1"),
        ];
        versions.sort_by(|a, b| b.cmp(a));
        let order: Vec<&str> = versions.iter().map(RuntimeVersion::as_str).collect();
        assert_eq!(
            order,
            ["8.10.1", "8.10.0", "8.9.0", "8.4.0-rc1", "8.3.14", "7.4.33"]
        );
    }

    #[test]
    fn lines_match_whole_numbers_only() {
        assert!(v("8.3.14").is_on_line("8.3"));
        assert!(v("8.3.14").is_on_line("8"));
        assert!(v("8.3.14").is_on_line("8.3.14"));
        assert!(!v("8.30.1").is_on_line("8.3"), "8.30 is not the 8.3 line");
        assert!(!v("18.3.1").is_on_line("8"));
        assert!(!v("8.3.14").is_on_line("8.4"));
        assert!(!v("8.3.14").is_on_line(""));
        assert!(!v("8.3.14").is_on_line("8.x"));
        assert_eq!(v("8.3.14").major_minor(), "8.3");
        assert_eq!(v("24.22.2").major_minor(), "24.22");
    }

    #[test]
    fn dates_parse_strictly_and_compare() {
        assert_eq!(CalDate::parse("2027-12-31"), CalDate::new(2027, 12, 31));
        assert!(CalDate::parse("2027-12-31") > CalDate::parse("2027-12-30"));
        assert!(CalDate::parse("2028-01-01") > CalDate::parse("2027-12-31"));
        for ok in ["2024-02-29", "2000-02-29", "2026-02-28"] {
            assert!(CalDate::parse(ok).is_some(), "{ok}");
        }
        for bad in [
            "",
            "2027",
            "2027-12",
            "2027-13-01",
            "2027-00-10",
            "2027-02-29",
            "1900-02-29",
            "2027-04-31",
            "2027-1-1",
            "27-12-31",
            "2027-12-31-01",
            "2027/12/31",
            "2027-12-3x",
        ] {
            assert!(CalDate::parse(bad).is_none(), "{bad:?}");
        }
    }
}
