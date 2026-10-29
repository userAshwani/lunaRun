# lunaRun — Decision Log

Short architecture decision records. Add new ones at the bottom: `D-0NN`, date, status
(`accepted` / `superseded by D-0xx`), context, decision, consequences. Never delete a record. Supersede it.

---

### D-001 — Desktop framework: Tauri 2 (2026-10-29, accepted)
**Context:** We need one codebase for Windows/macOS/Linux, with installers built by GitHub Actions. The app sits in the tray all day.
**Options:** Electron (TypeScript only, ~100 MB+ installer, ~150–300 MB RAM) · Tauri 2 (Rust + TypeScript, ~10 MB, low RAM) · native per OS (rejected: several codebases).
**Decision:** Tauri 2. Rust is well suited to process supervision, archives and OS APIs (Job Objects, ports). The official `tauri-action` builds and releases all OSes, and there is a built-in signed updater.
**Consequences:** Two languages (Rust backend, TS frontend), but one codebase and one pipeline. Webview differences (WebView2 / WKWebView / WebKitGTK) need testing on each OS.

### D-002 — UI-free core crate (2026-10-29, accepted)
**Decision:** All logic lives in `crates/luna-core`, and `src-tauri` is a thin adapter.
**Consequences:** Unit-testable without a GUI. The `luna` CLI (Phase 2) reuses the core.

### D-003 — Runtimes are downloaded, not bundled (2026-10-29, accepted)
**Decision:** The installer contains only lunaRun. Runtimes are fetched on demand from upstream sources listed in the registry.
**Consequences:** Small installer. Users can pick any version. We avoid most redistribution obligations (GPL MySQL/MariaDB). The first run needs internet.

### D-004 — Web server for MVP: Caddy (2026-10-29, accepted)
**Context:** Nginx has official binaries only for Windows. Apache on Windows comes from third parties.
**Decision:** Caddy 2. It has official binaries for every OS/arch, simple config, a reload API, `php_fastcgi` built in, and a local CA for HTTPS later.
**Consequences:** Nginx/Apache become optional per-site engines in Phase 3.

### D-005 — PHP binaries on macOS/Linux: static-php-cli builds (2026-10-29, accepted)
**Context:** PHP publishes official binaries only for Windows.
**Decision:** Use static-php-cli builds (upstream prebuilt first, our own CI builds if needed; PRD Q3).
**Consequences:** One self-contained binary per version. Fixed extension set per build. No shared extensions (affects Xdebug, Q5). PHP < 8.1 is not offered on macOS/Linux in v1.

### D-006 — State in JSON files, filesystem is the source of truth for installs (2026-10-29, accepted)
**Decision:** Small JSON files with atomic writes and schema versions. Install receipts sit inside each runtime directory. No SQLite.
**Consequences:** Human-readable, easy to debug and back up. Revisit if data grows (e.g. request logs).

### D-007 — License: Apache-2.0 (2026-10-29, accepted)
**Options:** MIT (simplest) · Apache-2.0 (permissive + explicit patent grant + no trademark grant) · GPL-3.0 (forks must stay open, which discourages companies from contributing).
**Decision:** Apache-2.0. It is permissive enough to drive adoption, the patent grant protects contributors and users, and §6 grants no rights to the "lunaRun" name or logo, so forks can't pass themselves off as the project.
**Consequences:** Contributions are accepted under the same license (inbound = outbound, stated in CONTRIBUTING.md). Add a `NOTICE` file if third-party Apache code is ever included.

### D-008 — No telemetry (2026-10-29, accepted)
**Decision:** No analytics or crash reporting that phones home. Diagnostics are copied manually by the user.

### D-009 — Registry decoupled from app releases (2026-10-29, accepted)
**Decision:** The available-versions list is a separately hosted, generated `registry.json` (with a bundled fallback).
**Consequences:** New PHP/MySQL/Node releases appear without an app release. It must be hosted publicly even while the app repo is private (RUNTIMES §5).

### D-010 — Never write into user projects without consent (2026-10-29, accepted)
**Decision:** Detection is read-only. Any write (`.env`, `.lunarun.json`) is an explicit user action with a preview and a backup.
