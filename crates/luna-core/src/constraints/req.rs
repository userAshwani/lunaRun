//! The one model both syntaxes parse into: a requirement is a **union of intervals** over versions.
//!
//! `^8.2` is `[8.2.0, 9.0.0)`, `^7.4 || ^8.0` is two intervals that touch and become one, `!=8.2.0` is the two sides of a hole.
//! Intersection and union are then plain set operations, which is what resolution needs ("the requirement of composer.json
//! intersected with that of the lock file"), and an empty result is how "nothing satisfies this" shows up.
//!
//! **Pre-releases.** A pre-release such as `8.5.0-rc1` is ordered before its release (`RuntimeVersion`'s order), so it falls
//! inside intervals like any other version. It *matches* only when the requirement mentions a pre-release **of the same
//! `major.minor.patch`** (`>=8.5.0-rc1`): node-semver's rule, and the policy of docs/PROJECT-DETECTION.md section 7 ("stable
//! only, unless the constraint explicitly allows pre-releases"). Exclusive upper bounds a parser derives (`<9.0.0` from
//! `^8.1`) are written `9.0.0-0`, the smallest pre-release, so that no pre-release of `9.0.0` slips in below the bound.

use std::cmp::Ordering;
use std::fmt;

use crate::registry::RuntimeVersion;

/// One end of an interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    Unbounded,
    Included(RuntimeVersion),
    Excluded(RuntimeVersion),
}

impl Bound {
    fn version(&self) -> Option<&RuntimeVersion> {
        match self {
            Self::Unbounded => None,
            Self::Included(v) | Self::Excluded(v) => Some(v),
        }
    }

    fn is_included(&self) -> bool {
        matches!(self, Self::Included(_))
    }
}

/// Which of two lower bounds starts first. An included bound at a version starts before an excluded one there.
fn cmp_lower(a: &Bound, b: &Bound) -> Ordering {
    match (a.version(), b.version()) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => x.cmp(y).then_with(|| b.is_included().cmp(&a.is_included())),
    }
}

/// Which of two upper bounds ends first. An excluded bound at a version ends before an included one there.
fn cmp_upper(a: &Bound, b: &Bound) -> Ordering {
    match (a.version(), b.version()) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y).then_with(|| a.is_included().cmp(&b.is_included())),
    }
}

/// `0.0.0-0`: below every other version. An interval that ends (exclusively) there holds nothing.
fn is_minimum(v: &RuntimeVersion) -> bool {
    v.numbers().iter().all(|n| *n == 0) && v.pre_identifiers() == ["0"]
}

/// All versions between a lower and an upper bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    pub lower: Bound,
    pub upper: Bound,
}

impl Interval {
    pub fn new(lower: Bound, upper: Bound) -> Self {
        Self { lower, upper }
    }

    pub fn is_empty(&self) -> bool {
        if let Bound::Excluded(top) = &self.upper
            && is_minimum(top)
        {
            return true;
        }
        match (self.lower.version(), self.upper.version()) {
            (Some(low), Some(high)) => match low.cmp(high) {
                Ordering::Greater => true,
                Ordering::Equal => !(self.lower.is_included() && self.upper.is_included()),
                Ordering::Less => false,
            },
            _ => false,
        }
    }

    pub fn contains(&self, v: &RuntimeVersion) -> bool {
        let above = match &self.lower {
            Bound::Unbounded => true,
            Bound::Included(low) => v >= low,
            Bound::Excluded(low) => v > low,
        };
        let below = match &self.upper {
            Bound::Unbounded => true,
            Bound::Included(high) => v <= high,
            Bound::Excluded(high) => v < high,
        };
        above && below
    }

    fn intersect(&self, other: &Self) -> Option<Self> {
        let lower = if cmp_lower(&self.lower, &other.lower) == Ordering::Less {
            other.lower.clone()
        } else {
            self.lower.clone()
        };
        let upper = if cmp_upper(&self.upper, &other.upper) == Ordering::Greater {
            other.upper.clone()
        } else {
            self.upper.clone()
        };
        let both = Self { lower, upper };
        (!both.is_empty()).then_some(both)
    }

    /// Do the two share a point, or touch so that together they have no hole? (`self` starts no later than `next`.)
    ///
    /// `[7.4, 8.0.0-0)` and `[8.0.0, 9.0.0-0)` touch: between `8.0.0-0` and `8.0.0` lie only pre-releases of 8.0.0, which
    /// need to be named to match anyway, so `^7.4 || ^8.0` is one range.
    fn joins(&self, next: &Self) -> bool {
        match (self.upper.version(), next.lower.version()) {
            (None, _) | (_, None) => true,
            (Some(end), Some(start)) => match start.cmp(end) {
                Ordering::Less => true,
                Ordering::Equal => self.upper.is_included() || next.lower.is_included(),
                Ordering::Greater => {
                    matches!(self.upper, Bound::Excluded(_))
                        && is_marker(end)
                        && next.lower.is_included()
                        && !start.is_prerelease()
                        && release_key(start) == release_key(end)
                }
            },
        }
    }
}

/// The numbers of a version as a `major.minor.patch` key: how a pre-release is told to belong to a mentioned release.
pub(super) fn release_key(v: &RuntimeVersion) -> [u64; 3] {
    let n = v.numbers();
    let at = |i: usize| n.get(i).copied().unwrap_or(0);
    [at(0), at(1), at(2)]
}

/// A requirement on a version: a union of intervals, normalised (sorted, no empty interval, touching ones merged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReq {
    intervals: Vec<Interval>,
    /// The `major.minor.patch` of every pre-release the requirement was written with, sorted: the releases whose
    /// pre-releases may match.
    prerelease_of: Vec<[u64; 3]>,
}

impl VersionReq {
    /// Normalise `intervals` into a requirement. `prerelease_of` names the releases whose pre-releases are allowed.
    pub(super) fn from_parts(intervals: Vec<Interval>, mut prerelease_of: Vec<[u64; 3]>) -> Self {
        let mut sorted: Vec<Interval> = intervals.into_iter().filter(|i| !i.is_empty()).collect();
        sorted.sort_by(|a, b| cmp_lower(&a.lower, &b.lower));
        let mut merged: Vec<Interval> = Vec::with_capacity(sorted.len());
        for next in sorted {
            match merged.last_mut() {
                Some(last) if last.joins(&next) => {
                    if cmp_upper(&last.upper, &next.upper) == Ordering::Less {
                        last.upper = next.upper;
                    }
                }
                _ => merged.push(next),
            }
        }
        prerelease_of.sort_unstable();
        prerelease_of.dedup();
        Self {
            intervals: merged,
            prerelease_of,
        }
    }

    /// Every version (`*`).
    pub fn any() -> Self {
        Self::from_parts(
            vec![Interval::new(Bound::Unbounded, Bound::Unbounded)],
            Vec::new(),
        )
    }

    /// No version at all.
    pub fn none() -> Self {
        Self::from_parts(Vec::new(), Vec::new())
    }

    /// Exactly `v`.
    pub fn exact(v: RuntimeVersion) -> Self {
        let prerelease = v.is_prerelease().then(|| release_key(&v));
        Self::from_parts(
            vec![Interval::new(
                Bound::Included(v.clone()),
                Bound::Included(v),
            )],
            prerelease.into_iter().collect(),
        )
    }

    /// The intervals, in order, with a gap between each.
    pub fn intervals(&self) -> &[Interval] {
        &self.intervals
    }

    /// Does `v` satisfy the requirement? A pre-release only when the requirement was written with a pre-release of the
    /// same `major.minor.patch`.
    pub fn matches(&self, v: &RuntimeVersion) -> bool {
        if v.is_prerelease() && !self.prerelease_of.contains(&release_key(v)) {
            return false;
        }
        self.intervals.iter().any(|interval| interval.contains(v))
    }

    /// No version can satisfy it (for example `>=8.3 <8.2`, or the intersection of `^7.4` and `^8.0`).
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// Every version satisfies it (`*`).
    pub fn is_any(&self) -> bool {
        matches!(
            self.intervals.as_slice(),
            [Interval {
                lower: Bound::Unbounded,
                upper: Bound::Unbounded
            }]
        )
    }

    /// Was a pre-release written in the requirement, so that some pre-release can match?
    pub fn allows_prereleases(&self) -> bool {
        !self.prerelease_of.is_empty()
    }

    /// Both requirements at once: the versions that satisfy `self` **and** `other`. Pre-releases are allowed for the
    /// releases either one names.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        let mut both = Vec::new();
        for a in &self.intervals {
            for b in &other.intervals {
                if let Some(common) = a.intersect(b) {
                    both.push(common);
                }
            }
        }
        Self::from_parts(both, self.merged_prereleases(other))
    }

    /// Either requirement: the versions that satisfy `self` **or** `other`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut all = self.intervals.clone();
        all.extend(other.intervals.iter().cloned());
        Self::from_parts(all, self.merged_prereleases(other))
    }

    fn merged_prereleases(&self, other: &Self) -> Vec<[u64; 3]> {
        let mut keys = self.prerelease_of.clone();
        keys.extend(other.prerelease_of.iter().copied());
        keys
    }

    /// The highest of `versions` that satisfies the requirement.
    pub fn highest_match<'a>(
        &self,
        versions: impl IntoIterator<Item = &'a RuntimeVersion>,
    ) -> Option<&'a RuntimeVersion> {
        versions.into_iter().filter(|v| self.matches(v)).max()
    }
}

/// A version as shown in a bound: the `-0` a parser adds to an exclusive upper bound is an implementation detail.
fn shown(v: &RuntimeVersion) -> &str {
    if is_marker(v) {
        v.as_str().strip_suffix("-0").unwrap_or_else(|| v.as_str())
    } else {
        v.as_str()
    }
}

/// `9.0.0-0`: the smallest pre-release of a release.
fn is_marker(v: &RuntimeVersion) -> bool {
    v.pre_identifiers() == ["0"]
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let (Bound::Included(low), Bound::Included(high)) = (&self.lower, &self.upper)
            && low == high
        {
            return write!(f, "={}", shown(low));
        }
        let mut parts: Vec<String> = Vec::new();
        match &self.lower {
            Bound::Unbounded => {}
            Bound::Included(v) => parts.push(format!(">={}", shown(v))),
            Bound::Excluded(v) => parts.push(format!(">{}", shown(v))),
        }
        match &self.upper {
            Bound::Unbounded => {}
            Bound::Included(v) => parts.push(format!("<={}", shown(v))),
            Bound::Excluded(v) => parts.push(format!("<{}", shown(v))),
        }
        if parts.is_empty() {
            f.write_str("*")
        } else {
            f.write_str(&parts.join(" "))
        }
    }
}

impl fmt::Display for VersionReq {
    /// `>=8.2.0 <9.0.0`, `>=7.4.0 <8.0.0 || >=8.1.0`, `*`, or `no version`: what the checklist shows.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.intervals.is_empty() {
            return f.write_str("no version");
        }
        let text: Vec<String> = self.intervals.iter().map(ToString::to_string).collect();
        f.write_str(&text.join(" || "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> RuntimeVersion {
        RuntimeVersion::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    fn range(low: Option<(&str, bool)>, high: Option<(&str, bool)>) -> Interval {
        let lower = match low {
            None => Bound::Unbounded,
            Some((t, true)) => Bound::Included(v(t)),
            Some((t, false)) => Bound::Excluded(v(t)),
        };
        let upper = match high {
            None => Bound::Unbounded,
            Some((t, true)) => Bound::Included(v(t)),
            Some((t, false)) => Bound::Excluded(v(t)),
        };
        Interval::new(lower, upper)
    }

    fn req(intervals: Vec<Interval>) -> VersionReq {
        VersionReq::from_parts(intervals, Vec::new())
    }

    #[test]
    fn an_interval_contains_what_lies_between_its_bounds() {
        let half_open = range(Some(("8.2.0", true)), Some(("9.0.0", false)));
        assert!(
            half_open.contains(&v("8.2.0")),
            "the lower bound is included"
        );
        assert!(half_open.contains(&v("8.9.9")));
        assert!(
            !half_open.contains(&v("9.0.0")),
            "the upper bound is excluded"
        );
        assert!(!half_open.contains(&v("8.1.99")));
        let open = range(Some(("1.0.0", false)), Some(("1.0.0", false)));
        assert!(
            open.is_empty(),
            "nothing lies strictly between a version and itself"
        );
        let point = range(Some(("1.0.0", true)), Some(("1.0.0", true)));
        assert!(point.contains(&v("1.0.0")) && !point.is_empty());
        assert!(!range(Some(("2.0.0", true)), Some(("1.0.0", true))).contains(&v("1.5.0")));
        assert!(range(Some(("2.0.0", true)), Some(("1.0.0", true))).is_empty());
        assert!(range(None, None).contains(&v("0.0.1")));
    }

    #[test]
    fn four_numbers_and_padding_order_as_versions_do() {
        let interval = range(Some(("1.2.3.4", true)), Some(("1.2.4", false)));
        assert!(interval.contains(&v("1.2.3.4")) && interval.contains(&v("1.2.3.5")));
        assert!(!interval.contains(&v("1.2.3")) && !interval.contains(&v("1.2.4")));
        assert!(
            range(Some(("8.1", true)), None).contains(&v("8.1.0")),
            "8.1 is 8.1.0"
        );
    }

    #[test]
    fn overlapping_and_touching_intervals_become_one() {
        let a = range(Some(("7.4.0", true)), Some(("8.0.0", false)));
        let b = range(Some(("8.0.0", true)), Some(("9.0.0", false)));
        assert_eq!(
            req(vec![a.clone(), b.clone()]).intervals().len(),
            1,
            "[7.4, 8) and [8, 9) have no hole"
        );
        assert_eq!(
            req(vec![b, a]).to_string(),
            ">=7.4.0 <9.0.0",
            "the order they are given in does not matter"
        );
        let c = range(Some(("1.0.0", true)), Some(("2.0.0", false)));
        let d = range(Some(("2.0.0", false)), Some(("3.0.0", true)));
        assert_eq!(
            req(vec![c, d]).intervals().len(),
            2,
            "(2.0.0 is in neither: a hole of one version)"
        );
        let e = range(Some(("1.0.0", true)), Some(("5.0.0", true)));
        let f = range(Some(("2.0.0", true)), Some(("3.0.0", true)));
        assert_eq!(
            req(vec![e, f]).to_string(),
            ">=1.0.0 <=5.0.0",
            "one inside the other"
        );
        let g = range(Some(("1.0.0", true)), Some(("2.0.0", true)));
        let h = range(Some(("2.0.0", false)), None);
        assert_eq!(
            req(vec![g, h]).to_string(),
            ">=1.0.0",
            "[1, 2] and (2, ..) have no hole"
        );
    }

    #[test]
    fn empty_intervals_are_dropped_and_the_empty_requirement_says_so() {
        let empty = req(vec![range(Some(("2.0.0", true)), Some(("1.0.0", true)))]);
        assert!(empty.is_empty());
        assert_eq!(empty.to_string(), "no version");
        assert_eq!(empty, VersionReq::none());
        assert!(!empty.matches(&v("1.5.0")));
        // `<0.0.0-0` is how node-semver writes "nothing".
        let nothing = req(vec![Interval::new(
            Bound::Unbounded,
            Bound::Excluded(v("0.0.0-0")),
        )]);
        assert!(nothing.is_empty());
    }

    #[test]
    fn any_and_none_and_exact() {
        assert!(VersionReq::any().is_any() && VersionReq::any().matches(&v("24.22.2")));
        assert_eq!(VersionReq::any().to_string(), "*");
        assert!(!VersionReq::none().is_any());
        let exact = VersionReq::exact(v("8.2.1"));
        assert!(exact.matches(&v("8.2.1")) && exact.matches(&v("8.2.1.0")));
        assert!(!exact.matches(&v("8.2.2")) && !exact.matches(&v("8.2.0")));
        assert_eq!(exact.to_string(), "=8.2.1");
    }

    #[test]
    fn a_prerelease_matches_only_a_requirement_that_names_one_of_the_same_release() {
        let stable = req(vec![range(Some(("8.5.0", true)), Some(("9.0.0", false)))]);
        assert!(stable.matches(&v("8.5.0")) && stable.matches(&v("8.6.1")));
        assert!(
            !stable.matches(&v("8.5.1-rc1")),
            "inside the interval, but nobody asked for pre-releases"
        );
        assert!(!stable.allows_prereleases());

        let wants_rc = VersionReq::from_parts(
            vec![range(Some(("8.5.0-rc1", true)), Some(("9.0.0", false)))],
            vec![[8, 5, 0]],
        );
        assert!(wants_rc.allows_prereleases());
        assert!(wants_rc.matches(&v("8.5.0-rc1")) && wants_rc.matches(&v("8.5.0-rc2")));
        assert!(
            !wants_rc.matches(&v("8.5.0-beta1")),
            "older than the one it starts at"
        );
        assert!(
            !wants_rc.matches(&v("8.6.0-rc1")),
            "a pre-release of another release"
        );
        assert!(wants_rc.matches(&v("8.5.0")));
        assert!(VersionReq::exact(v("8.5.0-rc1")).matches(&v("8.5.0-rc1")));
    }

    #[test]
    fn the_smallest_prerelease_keeps_every_prerelease_of_the_bound_out() {
        // `^8.1` is [8.1.0, 9.0.0-0): `9.0.0-rc1` is above the bound, so it cannot slip in even if 9.0.0 were named.
        let caret = VersionReq::from_parts(
            vec![range(Some(("8.1.0", true)), Some(("9.0.0-0", false)))],
            vec![[9, 0, 0]],
        );
        assert!(caret.matches(&v("8.9.9")));
        assert!(
            !caret.matches(&v("9.0.0-rc1"))
                && !caret.matches(&v("9.0.0"))
                && !caret.matches(&v("9.0.0-0"))
        );
        assert_eq!(
            caret.to_string(),
            ">=8.1.0 <9.0.0",
            "the marker is not shown"
        );
    }

    #[test]
    fn intersection_and_union_follow_the_set_laws_on_a_grid_of_versions() {
        // A deterministic spread of bounds and versions: every law is checked on every combination.
        let points = [
            "0.0.1", "1.0.0", "1.5.0", "2.0.0", "2.5.0", "3.0.0", "4.0.0",
        ];
        let mut intervals = vec![range(None, None)];
        for (i, low) in points.iter().enumerate() {
            for high in &points[i..] {
                for (li, hi) in [(true, true), (true, false), (false, true), (false, false)] {
                    intervals.push(range(Some((low, li)), Some((high, hi))));
                }
            }
            intervals.push(range(Some((low, true)), None));
            intervals.push(range(None, Some((low, false))));
        }
        let versions: Vec<RuntimeVersion> = [
            "0.0.0", "0.0.1", "0.5.0", "1.0.0", "1.2.0", "1.5.0", "1.9.9", "2.0.0", "2.2.0",
            "2.5.0", "2.7.0", "3.0.0", "3.5.0", "4.0.0", "4.0.1", "9.9.9",
        ]
        .iter()
        .map(|t| v(t))
        .collect();
        let reqs: Vec<VersionReq> = intervals
            .iter()
            .step_by(3)
            .map(|i| req(vec![i.clone()]))
            .chain([
                req(vec![
                    range(Some(("1.0.0", true)), Some(("2.0.0", false))),
                    range(Some(("3.0.0", true)), None),
                ]),
                req(vec![
                    range(None, Some(("1.5.0", true))),
                    range(Some(("2.5.0", false)), Some(("4.0.0", true))),
                ]),
            ])
            .collect();
        for a in &reqs {
            assert_eq!(
                a.intersect(a),
                *a,
                "a requirement intersected with itself is itself: {a}"
            );
            assert_eq!(a.union(a), *a, "and united with itself: {a}");
            assert_eq!(a.intersect(&VersionReq::any()), *a);
            assert_eq!(a.union(&VersionReq::none()), *a);
            assert!(a.intersect(&VersionReq::none()).is_empty());
            for b in &reqs {
                let (both, either) = (a.intersect(b), a.union(b));
                assert_eq!(
                    both,
                    b.intersect(a),
                    "intersection is commutative: {a} and {b}"
                );
                assert_eq!(either, b.union(a), "union is commutative: {a} and {b}");
                for version in &versions {
                    let (in_a, in_b) = (a.matches(version), b.matches(version));
                    assert_eq!(
                        both.matches(version),
                        in_a && in_b,
                        "{version} in ({a}) and ({b})"
                    );
                    assert_eq!(
                        either.matches(version),
                        in_a || in_b,
                        "{version} in ({a}) or ({b})"
                    );
                }
                // The result is normalised: sorted, with a real hole between neighbours.
                for pair in either.intervals().windows(2) {
                    assert!(
                        !pair[0].joins(&pair[1]),
                        "touching intervals were not merged in {either}"
                    );
                    assert_eq!(cmp_lower(&pair[0].lower, &pair[1].lower), Ordering::Less);
                }
            }
        }
    }

    #[test]
    fn the_highest_match_is_the_newest_version_that_satisfies() {
        let list: Vec<RuntimeVersion> =
            ["7.4.33", "8.1.34", "8.3.14", "8.2.28", "8.4.0-rc1", "8.3.9"]
                .iter()
                .map(|t| v(t))
                .collect();
        let wanted = req(vec![range(Some(("8.1.0", true)), Some(("8.3.20", false)))]);
        assert_eq!(wanted.highest_match(&list), Some(&list[2]));
        assert_eq!(VersionReq::none().highest_match(&list), None);
        assert_eq!(
            VersionReq::any().highest_match(&list),
            Some(&list[2]),
            "a pre-release is not the highest match"
        );
        let nothing_in_range = req(vec![range(Some(("9.0.0", true)), None)]);
        assert_eq!(nothing_in_range.highest_match(&list), None);
    }

    #[test]
    fn display_reads_like_the_constraint_it_came_from() {
        let two = req(vec![
            range(Some(("7.4.0", true)), Some(("8.0.0", false))),
            range(Some(("8.1.0", true)), None),
        ]);
        assert_eq!(two.to_string(), ">=7.4.0 <8.0.0 || >=8.1.0");
        assert_eq!(
            req(vec![range(None, Some(("8.4.0", false)))]).to_string(),
            "<8.4.0"
        );
        assert_eq!(
            req(vec![range(Some(("8.1.0", false)), Some(("8.3.5", true)))]).to_string(),
            ">8.1.0 <=8.3.5"
        );
    }
}
