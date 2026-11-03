//! Installed runtimes and what is known about each kind of runtime (FR-RT-01..04, FR-RT-13).
//!
//! - [`installed`]: the `install.json` receipt and the read-only listing of what is installed.
//! - [`php_config`] and [`codesign`]: what PHP needs after extraction (`php.ini`, CA bundle, user `conf.d`, macOS signing).
//! - [`prerequisites`]: what a runtime needs that lunaRun cannot install (the Visual C++ runtime on Windows).
//! - [`uninstall`]: removing a version, keeping database data unless asked.
//! - [`install`]: the install flow (download, extract, verify, atomic rename) and startup cleanup.
//! - [`adapter`] and [`adapters`]: the `RuntimeAdapter` trait and one adapter per runtime (layout, version
//!   check). The install flow itself (download, extract, post-install, atomic rename) comes next and
//!   uses these pieces.

pub mod adapter;
pub mod adapters;
pub mod codesign;
pub mod install;
pub mod installed;
pub mod php_config;
pub mod prerequisites;
pub mod uninstall;

pub use adapter::{
    Binary, InstallCtx, RuntimeAdapter, VERIFY_TIMEOUT, VersionCommand, VersionInfo,
    run_version_command,
};
pub use adapters::{adapter_for, adapters};
pub use install::{
    DISK_FACTOR, FetchRequest, Fetcher, FreeSpace, Installer, Prerequisites, cleanup_incomplete,
    downloader_fetcher,
};
pub use installed::{
    InstallReceipt, InstalledRuntime, InstalledView, RECEIPT_FILE, ReceiptArtifact, list_installed,
};
pub use php_config::{
    CA_BUNDLE, PhpConfigPaths, PhpSettings, php_config_paths, php_environment, write_php_config,
};
pub use uninstall::{RuntimeUsage, UninstallOptions, UninstallReport, Uninstaller, UsageChecker};
