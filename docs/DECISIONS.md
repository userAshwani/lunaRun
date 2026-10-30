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

### D-011 — Frontend router: React Router, hash mode (2026-10-29, accepted)
**Context:** The app has about six sidebar pages and no URL bar, SEO or server rendering. Candidates: React Router 8, TanStack Router 1.x, wouter.
**Decision:** `react-router` (declarative mode) with a **hash** router (`createHashRouter`). The hash keeps the current page across a dev reload and works under Tauri's custom protocol without server-side fallback rules.
**Why not TanStack Router:** its main benefit (type-safe routes via code generation) adds a build step and concepts we don't need for this few routes. React Router is the most widely known, which helps contributors and AI agents.
**Consequences:** Route paths are plain strings, kept in one `src/app/routes.ts` module to avoid typos. Revisit if routing grows (nested data loading, many params).

### D-012 — Rust → TypeScript types: ts-rs (2026-10-29, accepted)
**Context:** IPC types must not be hand-duplicated (CLAUDE.md rule 11). `ts-rs` 12.0.1 is stable. `specta` / `tauri-specta` are still on 2.0 release candidates (`2.0.0-rc.x`), and their 1.x stable lines target Tauri v1.
**Decision:** `ts-rs`. IPC structs/enums in `luna-core` derive `Serialize + Deserialize + TS` with `#[serde(rename_all = "camelCase")]` and `#[ts(export)]`. Generated files land in `src/bindings/` and are **committed**. `.cargo/config.toml` sets `TS_RS_EXPORT_DIR`. Files regenerate on `cargo test`. CI fails if `git diff --exit-code src/bindings` shows a difference (added with the CI task).
**Conventions** (verified by a spike):
- `u64`/`i64` become `bigint` in TS, but Tauri's JSON gives a plain `number`. Annotate such fields with `#[ts(type = "number")]` (byte counts, timestamps).
- Tagged enums use `#[serde(tag = "kind", rename_all = "camelCase")]` so TS gets a discriminated union.
- `src/lib/ipc.ts` holds thin typed `invoke` wrappers, one per command, importing only from `src/bindings/`.
**Consequences:** Command names and argument shapes are still written twice (Rust handler, TS wrapper), but payload types cannot drift. If that becomes painful, revisit `tauri-specta` once it reaches a stable 2.0 (supersede this record).

### D-013 — CI cost control while the repo is private (2026-10-30, superseded by D-015)
**Context:** Every working session ends with a push to `main`. On a private repo GitHub Free gives a monthly minute quota, and Windows runners count x2 and macOS x10. Running `cargo test` on both after every push would use the quota in a few weeks.
**Decision:** Per push and pull request, CI runs only on Ubuntu (lint, test, debug build) and skips docs-only changes. The Windows and macOS test matrix runs on `workflow_dispatch` (manual) and is exercised again by the release workflow on tags. Windows is also tested locally by the developer.
**Consequences:** A macOS-only break can go unnoticed until a manual run or a release tag. Run the matrix manually (`gh workflow run CI`) after touching OS-specific code. When the repo goes public (free minutes), change `test-os` to run on push to `main`.

### D-014 — HTTP client: reqwest with rustls and the OS trust store (2026-10-30, accepted)
**Context:** Runtimes are downloaded from several upstream hosts (D-003), on Windows, macOS and Linux, often behind corporate proxies and TLS-inspecting firewalls.
**Decision:** `reqwest` 0.13 with `default-features = false` and the `rustls`, `http2`, `stream` and `system-proxy` features. `rustls` brings the OS certificate store (`rustls-platform-verifier`), so a company root CA that Windows trusts also works here, and the aws-lc-rs crypto provider. System proxy settings are honoured. Redirects are checked by our own policy so an HTTPS URL can never lead to plain HTTP. Plain HTTP on loopback exists only behind `DownloadOptions::loopback_test()`, which the app never uses.
**Consequences:** No OpenSSL dependency and no system C library to install. aws-lc-rs built without extra tools on Windows (verified locally; Linux in CI). The cold build is about 1.7 minutes longer. If that becomes a problem, switch to `rustls-no-provider` with the `ring` provider.

### D-015 — Public repository: free CI, automatic publishing, permanent download names (2026-10-30, accepted)
**Context:** The owner made the repository public. That removes the private-repo minute budget behind D-013, and the README needs download links that keep working for every new version.
**Decision:** (1) The Windows and macOS Rust tests run on every push. (2) A tag publishes its release automatically, but only after all three platform builds succeed: the build creates a hidden draft and a final `publish` job (`scripts/finalize-release.mjs`) renames the installers to fixed names such as `lunaRun-windows-x64-setup.exe` and publishes. (3) The README links to `releases/latest/download/<fixed name>`. (4) Manual builds (Actions tab) default to all platforms and keep their installers as artifacts.
**Why rename instead of using `releaseAssetNamePattern`:** the pattern's `[arch]` and `[ext]` values differ per bundle type (`amd64`, `x86_64`, `.app.tar.gz`), which cannot be predicted without running a release. Renaming by a tested rule is deterministic and was verified against the real `v0.0.1` asset list.
**Consequences:** Files lose the version in their name (the version is the release tag and title). A bad tag can still publish a broken-but-complete build, so `check-versions` runs first and a release can be unpublished or deleted by the owner. The updater (Phase 2) must switch to `releaseAssetNamePattern` so `latest.json` matches.
