//! Version constraints: what `composer.json` says PHP must be, what `package.json` says Node must be (FR-PJ-01 to FR-PJ-03;
//! docs/PROJECT-DETECTION.md section 6).
//!
//! Two parsers, one model: [`parse_composer`] and [`parse_npm`] both produce a [`VersionReq`], a union of intervals over
//! [`RuntimeVersion`](crate::registry::RuntimeVersion)s, so requirements from different files can be **intersected**
//! (`composer.json` with every package of the lock file) and **united**, and "nothing satisfies this" is an empty result.
//!
//! Both are checked against the real implementations: the golden tables in `tests/fixtures/constraints/` are produced by
//! Composer's own semver library and by node-semver (`scripts/constraint-oracle/`), and `tests/constraints_oracle.rs` holds
//! these parsers to every answer in them. The deliberate differences are listed in `docs/DECISIONS.md` (D-034).
//!
//! A constraint that cannot be read is a [`ConstraintError`] with the position of the problem, never a panic, and never
//! "matches everything": a project whose requirement we cannot read must be told so, not given a guess.

mod composer;
mod npm;
mod req;

use std::fmt;

pub use composer::parse_composer;
pub use npm::parse_npm;
pub use req::{Bound, Interval, VersionReq};

/// A constraint that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintError {
    /// What is wrong, in a sentence a person can read.
    pub message: String,
    /// Where, as a byte offset into the text that was given (it points at the start of the part that is wrong).
    pub position: usize,
}

impl ConstraintError {
    pub(crate) fn new(message: impl Into<String>, position: usize) -> Self {
        Self {
            message: message.into(),
            position,
        }
    }
}

impl fmt::Display for ConstraintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at position {})", self.message, self.position)
    }
}

impl std::error::Error for ConstraintError {}

/// Something worth telling the user about a constraint that was read, but not obeyed to the letter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintWarning {
    /// A development branch (`dev-main`, `1.0.x-dev`, `dev-main as 8.1.0`): there is no version number to compare, so it
    /// does not restrict the version. Next to other alternatives it is left out; alone, the requirement is unconstrained.
    DevBranch(String),
}

/// A constraint that was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub req: VersionReq,
    pub warnings: Vec<ConstraintWarning>,
}
