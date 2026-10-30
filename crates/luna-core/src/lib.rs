//! lunaRun core: all business logic lives here and never depends on Tauri.
//! See docs/ARCHITECTURE.md for the module layout.

pub mod diagnostics;
pub mod error;
pub mod paths;
pub mod state;

pub use error::{ErrorCode, LunaError, Result};
pub use paths::LunaHome;

/// Product name. Kept in one place so a rename (PRD Q1) is cheap.
/// `src-tauri/tests/config_consistency.rs` fails if `tauri.conf.json` disagrees.
pub const PRODUCT_NAME: &str = "lunaRun";

/// Bundle identifier. Permanent once released: changing it breaks updates and data paths (PRD Q2).
pub const APP_IDENTIFIER: &str = "com.digitallybird.lunarun";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_is_reverse_dns() {
        assert!(APP_IDENTIFIER.starts_with("com.digitallybird."));
        assert!(!APP_IDENTIFIER.contains(char::is_whitespace));
    }

    #[test]
    fn product_name_is_set() {
        assert_eq!(PRODUCT_NAME, "lunaRun");
    }
}
