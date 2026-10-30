/// Commands the webview may call. Declaring them here makes Tauri generate one permission per
/// command (`allow-<name>`), so `capabilities/default.json` grants exactly these and nothing else.
const COMMANDS: &[&str] = &["app_info"];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(err) = tauri_build::try_build(attributes) {
        eprintln!("tauri build script failed: {err:#}");
        std::process::exit(1);
    }
}
