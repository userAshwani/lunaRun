//! The two worlds the project tests resolve in: what a Windows PC and a Mac have installed, and what the registry offers
//! each of them. Built here, not read from the live registry, so the answers do not move when upstream releases a new PHP.
//! Shared by `projects_golden.rs` and `projects_resolve.rs`.

// Not every test program uses every helper.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use luna_core::diagnostics::{Arch, Os};
use luna_core::projects::DetectionData;
use luna_core::registry::{AvailableVersion, Channel, RuntimeCatalog};
use luna_core::runtimes::{InstallReceipt, InstalledRuntime};

pub fn row(runtime: &str, version: &str, eol: bool, available: bool) -> AvailableVersion {
    let numbers: Vec<&str> = version.split('.').collect();
    let line = if runtime == "node" {
        numbers[0].to_owned()
    } else {
        format!("{}.{}", numbers[0], numbers[1])
    };
    AvailableVersion {
        runtime: runtime.to_owned(),
        version: version.to_owned(),
        line,
        channel: Channel::Stable,
        eol,
        eol_date: None,
        security_only: false,
        released_at: None,
        community: false,
        size: None,
        available,
        rosetta_fallback: false,
    }
}

pub fn catalog(id: &str, rows: Vec<AvailableVersion>) -> RuntimeCatalog {
    RuntimeCatalog {
        id: id.to_owned(),
        display_name: id.to_owned(),
        license: None,
        versions: rows,
    }
}

/// What the registry offers on 2026-11-14 for `os`. A Mac has the static PHP builds, which lag and stop at 8.1, and no
/// MariaDB.
pub fn catalogs(os: Os) -> Vec<RuntimeCatalog> {
    let win = os == Os::Windows;
    let php = if win {
        vec![
            row("php", "8.5.12", false, true),
            row("php", "8.4.27", false, true),
            row("php", "8.3.36", false, true),
            row("php", "8.2.35", false, true),
            row("php", "8.1.34", true, true),
            row("php", "8.0.30", true, true),
            row("php", "7.4.33", true, true),
        ]
    } else {
        vec![
            row("php", "8.5.8", false, true),
            row("php", "8.4.23", false, true),
            row("php", "8.3.32", false, true),
            row("php", "8.2.32", false, true),
            row("php", "8.1.31", true, true),
            row("php", "8.0.30", true, false),
            row("php", "7.4.33", true, false),
        ]
    };
    let node = vec![
        row("node", "26.13.1", false, true),
        row("node", "25.9.0", true, true),
        row("node", "24.22.3", false, true),
        row("node", "23.11.1", true, true),
        row("node", "22.23.5", false, true),
        row("node", "21.7.3", true, true),
        row("node", "20.20.2", true, true),
        row("node", "18.20.8", true, true),
    ];
    let mysql = vec![
        row("mysql", "9.7.4", false, true),
        row("mysql", "8.4.8", false, true),
        row("mysql", "8.0.40", true, true),
    ];
    let mariadb = vec![
        row("mariadb", "12.3.3", false, win),
        row("mariadb", "11.8.9", false, win),
        row("mariadb", "10.6.20", true, win),
    ];
    vec![
        catalog("php", php),
        catalog("node", node),
        catalog("mysql", mysql),
        catalog("mariadb", mariadb),
    ]
}

pub fn installed(os: Os) -> Vec<InstalledRuntime> {
    let list: &[(&str, &str)] = if os == Os::Windows {
        &[
            ("php", "8.3.14"),
            ("php", "8.1.30"),
            ("node", "22.11.0"),
            ("node", "20.11.1"),
            ("mysql", "8.4.3"),
        ]
    } else {
        &[("php", "8.3.32"), ("node", "22.11.0")]
    };
    list.iter()
        .map(|(runtime, version)| InstalledRuntime {
            runtime: (*runtime).to_owned(),
            version: (*version).to_owned(),
            dir: PathBuf::from(format!("/runtimes/{runtime}/{version}")),
            receipt: InstallReceipt::new(runtime, version, os, Arch::X86_64, false, &[]),
        })
        .collect()
}

pub fn data() -> DetectionData {
    DetectionData::bundled().unwrap()
}
