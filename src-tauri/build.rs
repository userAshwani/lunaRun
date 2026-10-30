/// Commands the webview may call. Declaring them here makes Tauri generate one permission per
/// command (`allow-<name>`), so `capabilities/default.json` grants exactly these and nothing else.
const COMMANDS: &[&str] = &["app_info", "jobs_list", "job_cancel"];

fn main() {
    // Unit tests that use Tauri's mock runtime need the Common Controls v6 manifest on Windows, or the
    // test executable cannot start. `-tests` limits this to test binaries: the app itself already gets
    // a manifest from tauri-build, and two would conflict.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR")
    {
        let manifest = std::path::Path::new(&dir).join("windows-test-manifest.xml");
        println!("cargo:rerun-if-changed=windows-test-manifest.xml");
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }

    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(err) = tauri_build::try_build(attributes) {
        eprintln!("tauri build script failed: {err:#}");
        std::process::exit(1);
    }
}
