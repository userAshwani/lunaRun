//! Typed errors shared by every luna-core operation and sent to the UI.
//!
//! The UI never shows `message` blindly: it maps `code` to a translated string and uses `message`
//! and `details` as technical context. Codes are part of the IPC contract (docs/ARCHITECTURE.md §6).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use ts_rs::TS;

/// Stable error identifiers. Add variants freely; never rename or remove one once released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ErrorCode {
    NotFound,
    PermissionDenied,
    Io,
    /// A path was rejected because it is not inside the directory lunaRun owns (NFR-04).
    PathOutsideRoot,
    /// A path or path component is malformed or unsafe (empty, `..`, separators, relative, …).
    InvalidPath,
    /// A state file was unreadable and has been moved aside (NFR-08).
    StateCorrupt,
    /// A state file was written by a newer lunaRun; it is left untouched.
    StateSchemaTooNew,
    /// A network request failed (offline, DNS, timeout, bad HTTP status, connection dropped).
    Network,
    /// A downloaded file did not match its published SHA-256 and was deleted (NFR-05).
    ChecksumMismatch,
    /// The user (or lunaRun) cancelled a long-running operation.
    Cancelled,
    /// A URL is malformed or not allowed (only HTTPS is accepted).
    InvalidUrl,
    /// An archive entry tried to escape the destination or is otherwise not allowed (zip-slip, symlink
    /// tricks, special files). Nothing outside the destination was touched.
    ArchiveUnsafe,
    /// An archive is truncated, damaged or uses an unsupported feature.
    ArchiveCorrupt,
    /// An archive would expand beyond the allowed size or entry count (decompression bomb).
    ArchiveTooLarge,
    /// The list of downloadable versions (the registry) is malformed or breaks a rule.
    RegistryInvalid,
    /// The registry was written for a newer lunaRun. Update the app.
    RegistryUnsupportedSchema,
    /// The registry has no download for this operating system and processor.
    NoArtifactForPlatform,
    /// A bug or an impossible state inside lunaRun.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize, TS)]
#[error("{message}")]
#[ts(export)]
pub struct LunaError {
    pub code: ErrorCode,
    /// English technical description, for logs and the diagnostics report.
    pub message: String,
    /// Structured context such as `{ "path": "…" }`. Never contains secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub details: Option<Value>,
}

pub type Result<T> = std::result::Result<T, LunaError>;

impl LunaError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

fn code_for(kind: io::ErrorKind) -> ErrorCode {
    match kind {
        io::ErrorKind::NotFound => ErrorCode::NotFound,
        io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        _ => ErrorCode::Io,
    }
}

impl From<io::Error> for LunaError {
    fn from(err: io::Error) -> Self {
        Self::new(code_for(err.kind()), err.to_string())
    }
}

/// Attaches the offending path to an I/O error, so messages are never just "access denied".
pub trait IoResultExt<T> {
    fn at_path(self, path: &Path) -> Result<T>;
}

impl<T> IoResultExt<T> for io::Result<T> {
    fn at_path(self, path: &Path) -> Result<T> {
        self.map_err(|err| {
            let shown = path.display().to_string();
            LunaError::new(code_for(err.kind()), format!("{shown}: {err}"))
                .with_details(json!({ "path": shown }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_screaming_snake_code_and_no_null_details() {
        let err = LunaError::new(ErrorCode::PathOutsideRoot, "nope");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            json!({ "code": "PATH_OUTSIDE_ROOT", "message": "nope" })
        );
    }

    #[test]
    fn details_round_trip() {
        let err = LunaError::new(ErrorCode::Io, "x").with_details(json!({ "path": "C:\\a b" }));
        let back: LunaError = serde_json::from_str(&serde_json::to_string(&err).unwrap()).unwrap();
        assert_eq!(back, err);
    }

    #[test]
    fn io_errors_map_to_stable_codes() {
        let not_found = LunaError::from(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(not_found.code, ErrorCode::NotFound);
        let denied = LunaError::from(io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(denied.code, ErrorCode::PermissionDenied);
        let other = LunaError::from(io::Error::other("boom"));
        assert_eq!(other.code, ErrorCode::Io);
    }

    #[test]
    fn at_path_names_the_path_in_message_and_details() {
        let err = Err::<(), _>(io::Error::from(io::ErrorKind::NotFound))
            .at_path(Path::new("D:/My Dir/ü.json"))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(err.message.contains("My Dir"));
        assert_eq!(err.details.unwrap()["path"], "D:/My Dir/ü.json");
    }
}
