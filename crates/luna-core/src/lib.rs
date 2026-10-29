//! lunaRun core: all business logic lives here and never depends on Tauri.
//! See docs/ARCHITECTURE.md for the module layout.

/// Product name. Kept in one place so a rename (PRD Q1) is cheap.
pub const PRODUCT_NAME: &str = "lunaRun";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_name_is_set() {
        assert_eq!(PRODUCT_NAME, "lunaRun");
    }
}
