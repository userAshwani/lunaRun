//! `tauri.conf.json` is static JSON and cannot read Rust constants, so this test keeps it in sync
//! with the single source of truth in `luna-core`.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::expect_used)]

use serde_json::Value;

fn tauri_conf() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let text = std::fs::read_to_string(path).expect("tauri.conf.json must be readable");
    serde_json::from_str(&text).expect("tauri.conf.json must be valid JSON")
}

#[test]
fn product_name_matches_core() {
    assert_eq!(tauri_conf()["productName"], luna_core::PRODUCT_NAME);
}

#[test]
fn window_title_matches_core() {
    assert_eq!(
        tauri_conf()["app"]["windows"][0]["title"],
        luna_core::PRODUCT_NAME
    );
}

#[test]
fn identifier_matches_core() {
    assert_eq!(tauri_conf()["identifier"], luna_core::APP_IDENTIFIER);
}
