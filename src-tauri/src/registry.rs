//! Loading the version list for the running app. Logic lives in `luna-core` (`RegistryHolder`); this only
//! supplies what Tauri owns: the download client, the configured address and the event to the webview.

use luna_core::LunaError;
use luna_core::LunaHome;
use luna_core::download::{DownloadOptions, Downloader};
use luna_core::registry::{RegistryHolder, RegistryStatus, registry_location_with};
use tauri::{AppHandle, Runtime};

use crate::events::emit_registry_updated;

/// Refresh the list and tell the webview. Used by the startup task and by the `registry_refresh` command.
pub async fn refresh_and_announce<R: Runtime>(
    app: &AppHandle<R>,
    holder: &RegistryHolder,
    home: &LunaHome,
) -> Result<RegistryStatus, LunaError> {
    let downloader = Downloader::new(DownloadOptions::default())?;
    // The environment wins, then the address in the settings (D-045), then the built-in one.
    let setting = luna_core::settings::load(home)?.registry_url;
    let status = holder
        .refresh(
            home,
            &downloader,
            &registry_location_with(setting.as_deref()),
        )
        .await?;
    emit_registry_updated(app, &status);
    Ok(status)
}
