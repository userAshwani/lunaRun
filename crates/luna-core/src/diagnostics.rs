//! Facts about the running app, shown on the Diagnostics page and copied into bug reports (FR-AP-04).
//! Nothing here may contain secrets.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::paths::LunaHome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Os {
    Windows,
    Macos,
    Linux,
    Unknown,
}

impl Os {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            "linux" => Self::Linux,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Arch {
    #[serde(rename = "x86_64")]
    #[ts(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    #[ts(rename = "aarch64")]
    Aarch64,
    #[serde(rename = "unknown")]
    #[ts(rename = "unknown")]
    Unknown,
}

impl Arch {
    pub fn current() -> Self {
        match std::env::consts::ARCH {
            "x86_64" => Self::X86_64,
            "aarch64" => Self::Aarch64,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub app_version: String,
    pub os: Os,
    pub arch: Arch,
    /// Absolute path of the lunaRun data directory (LUNA_HOME).
    pub luna_home: String,
    /// True when `LUNA_HOME` is set in the environment rather than using the OS default.
    pub luna_home_from_env: bool,
}

/// `app_version` comes from the app crate, not from luna-core, so it always matches the shipped binary.
pub fn app_info(app_version: &str, home: &LunaHome) -> AppInfo {
    AppInfo {
        app_version: app_version.to_owned(),
        os: Os::current(),
        arch: Arch::current(),
        luna_home: home.root().display().to_string(),
        luna_home_from_env: home.from_env(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn collects_version_platform_and_home() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("lüna home");
        let home = LunaHome::at(&root).unwrap();

        let info = app_info("1.2.3", &home);
        assert_eq!(info.app_version, "1.2.3");
        assert_eq!(info.luna_home, root.display().to_string());
        assert!(
            !info.luna_home_from_env,
            "LunaHome::at is an explicit path, not the env var"
        );
        // CI and developer machines are all 64-bit desktop platforms.
        assert_ne!(info.os, Os::Unknown);
        assert_ne!(info.arch, Arch::Unknown);
    }

    #[test]
    fn serializes_in_the_shape_the_ui_expects() {
        let info = AppInfo {
            app_version: "0.0.0".into(),
            os: Os::Macos,
            arch: Arch::Aarch64,
            luna_home: "/Users/z/Library/Application Support/lunaRun".into(),
            luna_home_from_env: true,
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({
                "appVersion": "0.0.0",
                "os": "macos",
                "arch": "aarch64",
                "lunaHome": "/Users/z/Library/Application Support/lunaRun",
                "lunaHomeFromEnv": true,
            })
        );
        assert_eq!(serde_json::to_value(Arch::X86_64).unwrap(), json!("x86_64"));
    }
}
