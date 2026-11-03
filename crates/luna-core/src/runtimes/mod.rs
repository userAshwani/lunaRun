//! Installed runtimes and what is known about each kind of runtime (FR-RT-01..04, FR-RT-13).
//!
//! - [`installed`]: the `install.json` receipt and the read-only listing of what is installed.
//! - [`adapter`] and [`adapters`]: the `RuntimeAdapter` trait and one adapter per runtime (layout, version
//!   check). The install flow itself (download, extract, post-install, atomic rename) comes next and
//!   uses these pieces.

pub mod adapter;
pub mod adapters;
pub mod installed;

pub use adapter::{
    Binary, InstallCtx, RuntimeAdapter, VERIFY_TIMEOUT, VersionCommand, VersionInfo,
    run_version_command,
};
pub use adapters::{adapter_for, adapters};
pub use installed::{
    InstallReceipt, InstalledRuntime, RECEIPT_FILE, ReceiptArtifact, list_installed,
};
