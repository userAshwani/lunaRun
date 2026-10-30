# lunaRun — Architecture

Related: [PRD](PRD.md) · [DECISIONS](DECISIONS.md) · [RUNTIMES](RUNTIMES.md) · [SERVICES](SERVICES.md) · [PROJECT-DETECTION](PROJECT-DETECTION.md)

## 1. Tech stack

| Layer | Choice | Why |
|---|---|---|
| Desktop shell | **Tauri 2** | One codebase for Win/mac/Linux, small installers (~10 MB), native webview, built-in updater, official GitHub Action for releases. See D-001. |
| Backend / core | **Rust** (stable; MSRV pinned in `rust-toolchain.toml`) | Reliable process supervision, file system work, archive extraction, cross-platform. |
| Async runtime | `tokio` | Process spawning, downloads, timers. |
| Frontend | **React + TypeScript (strict) + Vite** | Mainstream, well known to contributors and to AI agents. |
| UI kit | Tailwind CSS + shadcn/ui (Radix primitives) + lucide-react icons | Accessible primitives, no heavy UI framework. |
| Frontend data | TanStack Query (wrapping Tauri `invoke`) + small Zustand store for UI-only state | Caching, refetch, optimistic updates. |
| Routing | React Router (hash router), see D-011 | Simple, mainstream; no URL bar in a desktop app. |
| Package manager | **pnpm**; Node **24 LTS** for building the frontend | |
| Tests | `cargo test` (core), Vitest + Testing Library (frontend), Playwright later for E2E | |
| Lint/format | `cargo fmt`, `cargo clippy -D warnings`, ESLint, Prettier | |

**Rule:** use well-maintained crates and check their current stable versions when adding them. Do not pin
versions from memory. Suggested crates: `reqwest` (rustls), `sha2`, `zip`, `tar`, `flate2`, `xz2`,
`serde`/`serde_json`, `thiserror`, `anyhow` (only at the app edge), `tracing` + `tracing-appender`,
`directories`, `minijinja` (config templates), `uuid`, `time`/`chrono`, `notify` (file watching, Phase 2),
`sysinfo` (process lookup), `windows` crate (Job Objects, registry) under `cfg(windows)`.

## 2. Repository layout

```
lunaRun/
├─ .github/
│  ├─ workflows/            ci.yml, release.yml, (registry.yml — Phase 1/2)
│  ├─ ISSUE_TEMPLATE/
│  └─ PULL_REQUEST_TEMPLATE.md
├─ crates/
│  ├─ luna-core/            # UI-free library: ALL business logic lives here
│  │  ├─ src/
│  │  │  ├─ lib.rs
│  │  │  ├─ paths.rs        # LUNA_HOME layout, guarded fs ops (safe_remove etc.)
│  │  │  ├─ registry/       # registry fetch, cache, parse, signature verify
│  │  │  ├─ runtimes/       # install/uninstall, receipts, per-runtime adapters (php.rs, mysql.rs…)
│  │  │  ├─ download.rs     # resumable download + sha256
│  │  │  ├─ extract.rs      # zip / tar.gz / tar.xz with zip-slip protection
│  │  │  ├─ services/       # supervisor, service definitions, health checks, logs
│  │  │  ├─ ports.rs        # allocation + conflict detection
│  │  │  ├─ projects/       # project store, detection, resolution
│  │  │  ├─ constraints/    # Composer + npm-semver constraint parsing/matching
│  │  │  ├─ templates/      # php.ini, php-fpm.conf, my.cnf, Caddyfile (minijinja)
│  │  │  ├─ terminal.rs     # "open terminal here" with project PATH
│  │  │  ├─ settings.rs
│  │  │  ├─ state.rs        # atomic JSON persistence + schema migrations
│  │  │  ├─ events.rs       # event types emitted to the UI
│  │  │  └─ error.rs        # LunaError (typed, serialisable)
│  │  ├─ data/              # laravel-php.json, extension maps, bundled fallback registry
│  │  └─ tests/fixtures/    # sample projects for detection tests
│  └─ luna-cli/             # Phase 2: `luna` binary + shims (reuses luna-core)
├─ src-tauri/               # thin Tauri app: commands → luna-core, tray, window, updater
│  ├─ src/main.rs, commands/*.rs, tray.rs
│  ├─ capabilities/         # Tauri v2 permission files (least privilege)
│  ├─ icons/
│  └─ tauri.conf.json
├─ src/                     # React frontend
│  ├─ app/ (routes, layout)  ├─ features/ (runtimes, services, projects, settings, diagnostics)
│  ├─ components/ui/ (shadcn) ├─ lib/ipc.ts (typed invoke wrappers) ├─ i18n/
├─ docs/
├─ scripts/                 # bump-version, registry generator (Phase 1/2)
├─ Cargo.toml               # workspace: crates/*, src-tauri
├─ package.json, pnpm-lock.yaml, vite.config.ts, tsconfig.json
├─ rust-toolchain.toml, .editorconfig, .gitignore
├─ CLAUDE.md, README.md, CHANGELOG.md, CONTRIBUTING.md, SECURITY.md, LICENSE
```

**Layering rule:** `src-tauri` contains no business logic, only argument mapping, calls into
`luna-core`, and event forwarding. `luna-core` never imports Tauri. This keeps the core testable and
reusable by `luna-cli`.

## 3. Component overview

```
┌──────────────────────── Tauri app process ───────────────────────────┐
│  React UI  ──invoke()──►  src-tauri commands  ──►  luna-core          │
│     ▲                         │                    ├─ RegistryClient  │
│     └──── events (emit) ◄─────┘                    ├─ RuntimeManager  │
│                                                    ├─ Supervisor ─────┼──► child processes
│  Tray menu ─────────────────────────────────────►  ├─ ProjectService  │     php-cgi / php-fpm
│                                                    ├─ PortAllocator   │     mysqld / mariadbd
│                                                    └─ StateStore      │     caddy
└───────────────────────────────────────────────────────────────────────┘
                     │ HTTPS                                   │ files
                     ▼                                         ▼
        registry.json + upstream downloads                 LUNA_HOME
```

- **RegistryClient**: fetches `registry.json` (remote URL from settings → cached copy → bundled fallback).
- **RuntimeManager**: install/uninstall jobs, receipts, post-install steps, per-runtime adapters behind a `RuntimeAdapter` trait.
- **Supervisor**: owns all child processes; state machine, restart policy, log capture, graceful shutdown. See [SERVICES](SERVICES.md).
- **ProjectService**: project CRUD, detection, resolution, bindings, site config generation.
- **JobManager** (`jobs.rs`): runs long tasks (install = download, verify, extract) on a Tokio runtime handle. Jobs are
  identified by `job-N`, deduplicated by a key such as `php@8.3.14` while running, cancellable through a
  `CancellationToken`, and report to a `JobEventSink` (throttled to about 10 progress events per second per job, the final
  state always delivered). `src-tauri` supplies a sink that emits `job:progress` and `job:done` to the webview. Jobs live in
  memory only (the newest 50 finished ones stay listable) and do not survive a restart.
- **PortAllocator**: persistent port assignments plus live conflict checks.
- **StateStore**: atomic JSON read/write with schema versions.

### Runtime adapter trait (sketch)

```rust
pub trait RuntimeAdapter: Send + Sync {
    fn id(&self) -> RuntimeId;                       // "php", "mysql", ...
    fn post_install(&self, ctx: &InstallCtx) -> Result<()>;  // e.g. generate php.ini, codesign on macOS
    fn executables(&self, install: &InstalledRuntime) -> Vec<PathBuf>; // dirs to put on PATH
    fn service_specs(&self, install: &InstalledRuntime, cfg: &ServiceCfg) -> Vec<ServiceSpec>; // empty for node/composer
    fn verify(&self, install: &InstalledRuntime) -> Result<VersionInfo>; // run `php -v` etc.
}
```

Adding a runtime = add a registry entry + an adapter. No UI changes beyond generic lists.

## 4. LUNA_HOME layout

Default location (overridable by the first-run wizard or the `LUNA_HOME` env var):

| OS | Default |
|---|---|
| Windows | `%LOCALAPPDATA%\lunaRun` |
| macOS | `~/Library/Application Support/lunaRun` |
| Linux | `$XDG_DATA_HOME/lunarun` (fallback `~/.local/share/lunarun`) |

```
LUNA_HOME/
├─ runtimes/
│  ├─ php/8.3.14/            # extracted binaries + install.json receipt
│  ├─ php/8.4.2/
│  ├─ mysql/8.4.3/
│  ├─ node/24.11.0/
│  ├─ composer/2.8.4/composer.phar
│  └─ caddy/2.10.0/
├─ config/
│  ├─ settings.json
│  ├─ projects.json
│  ├─ services.json          # port assignments, autostart flags
│  ├─ php/8.3.14/php.ini     # generated
│  ├─ php/8.3.14/conf.d/99-user.ini   # user overrides, never overwritten
│  ├─ mysql/8.4/my.cnf
│  └─ caddy/Caddyfile        # generated
├─ data/
│  ├─ mysql-8.4/             # DB data dir per engine + line (FR-SV-09)
│  └─ mariadb-11.4/
├─ logs/                     # app.log, <service-id>.log (rotated)
├─ run/                      # pid files, sockets
├─ cache/registry.json       # last fetched registry
└─ downloads/                # *.part files (resumable), cleaned after install
```

Everything lunaRun creates lives here (NFR-04). `paths.rs` exposes the only functions allowed to
delete files, and they refuse any path that is not inside LUNA_HOME after canonicalisation.

## 5. Data model (persisted JSON)

All files carry `"schema": <int>`. `state.rs` runs migrations on load.

```jsonc
// config/projects.json
{
  "schema": 1,
  "projects": [{
    "id": "4b1e…",                 // uuid v4
    "name": "shop",
    "path": "D:\\Work\\shop",       // absolute, stored as given (no canonical lowercasing)
    "docRoot": "public",            // relative to path
    "framework": { "kind": "laravel", "version": "12.3.0" },
    "bindings": {
      "php":      { "mode": "line",  "value": "8.3" },     // or { "mode": "exact", "value": "8.3.14" }
      "node":     { "mode": "exact", "value": "24.11.0" },
      "database": { "engine": "mysql", "line": "8.4" }      // optional
    },
    "site": { "port": 8101, "domain": null, "https": false },
    "autoStart": false,
    "lastDetection": { "at": "2026-10-29T10:00:00Z", "hash": "…" },  // hash of inputs, to detect changes
    "createdAt": "…", "updatedAt": "…"
  }]
}
```

```jsonc
// runtimes/php/8.3.14/install.json  (receipt — the filesystem is the source of truth for installs)
{ "schema": 1, "runtime": "php", "version": "8.3.14", "variant": "nts-vs16",
  "os": "windows", "arch": "x86_64", "source": "https://…", "sha256": "…",
  "installedAt": "…", "managed": true }
```

A runtime directory without a valid receipt is treated as an incomplete install and cleaned up on start.

## 6. IPC contract (Tauri commands)

All commands return `Result<T, LunaError>`. A command only works once it is declared in `src-tauri/build.rs` (`COMMANDS`) and granted as `allow-<name>` in `src-tauri/capabilities/default.json`, so the webview can call exactly the listed commands and nothing else. `LunaError` is serialised as
`{ code: string, message: string, details?: object }`. `code` is a stable SCREAMING_SNAKE id
(`PORT_IN_USE`, `CHECKSUM_MISMATCH`, `RUNTIME_IN_USE`, `NOT_FOUND`, `NETWORK`, `PERMISSION_DENIED`, …)
that the UI maps to a translated message. Long work returns a `jobId` straight away and reports through events.

| Area | Command | Returns |
|---|---|---|
| Registry | `registry_refresh()` | `RegistryMeta` |
| | `registry_versions(runtime)` | `AvailableVersion[]` |
| Runtimes | `runtimes_installed(runtime?)` | `InstalledRuntime[]` |
| | `runtime_install(runtime, version, variant?)` | `jobId` |
| | `runtime_uninstall(runtime, version, deleteData)` | `()` |
| | `runtime_usage(runtime, version)` | `ProjectRef[]` (who uses it) |
| Jobs | `job_cancel(jobId)` / `jobs_list()` | |
| Services | `services_list()` | `ServiceStatus[]` |
| | `service_start(id)` / `service_stop(id)` / `service_restart(id)` | `()` |
| | `service_logs(id, tailLines)` | `LogLine[]` |
| Projects | `projects_list()` / `project_get(id)` | `Project` |
| | `project_add(path)` | `Project` + `DetectionReport` |
| | `project_detect(id)` | `DetectionReport` |
| | `project_update(id, patch)` | `Project` |
| | `project_remove(id)` | `()` (never deletes files on disk) |
| | `project_start(id)` / `project_stop(id)` | `()` |
| | `project_open(id, target: "browser"\|"terminal"\|"folder"\|"editor")` | `()` |
| | `project_apply_fix(id, fixId)` | `jobId` (e.g. install + bind recommended PHP) |
| Settings | `settings_get()` / `settings_set(patch)` | `Settings` |
| System | `ports_check(ports[])` | `PortStatus[]` |
| | `diagnostics()` | `DiagnosticsReport` |

**Events** (backend → UI, via `emit`):
`job:progress {jobId, phase, bytesDone, bytesTotal, message}`, `job:done {jobId, ok, error?}`,
`service:state {id, state, pid?, port?}`, `service:log {id, stream, line, ts}` (batched, throttled ≤ 10/s),
`registry:updated`, `project:changed {id}` (Phase 2 file watcher).

TypeScript types for these MUST be generated from Rust with `ts-rs` (D-012, output in `src/bindings/`) so the
two sides cannot drift.

## 7. Security model

- Tauri v2 **capabilities**: the webview gets only the custom commands above + required core plugins. No generic shell or fs plugin exposed to JS.
- No remote content in the webview; strict CSP; all assets bundled.
- Process spawning only from `luna-core`, only for executables inside LUNA_HOME (or registered external runtimes), with argument arrays (never a shell string).
- Downloads: HTTPS only, SHA-256 required, zip-slip guarded extraction, size limits.
- Elevation (Phase 2 hosts/CA): a tiny dedicated helper invoked with OS prompts (UAC `runas` / macOS `osascript … with administrator privileges` / `pkexec`), doing one fixed operation with validated input.
- Secrets: `.env` is read key-by-key for `DB_*`, `REDIS_*`, `MAIL_*` hints only. Values are never logged or sent.

## 8. Cross-platform rules

- Use `std::path::PathBuf` everywhere; never build paths with string concatenation or hard-coded `/` or `\`.
- OS-specific code goes in `#[cfg(target_os = "...")]` modules behind a common function (`platform::open_terminal`, `platform::kill_tree`, …).
- When writing paths into config files (Caddyfile, php.ini, my.cnf), use the format each tool expects. Windows paths in Caddyfile/my.cnf use forward slashes, and every path is quoted.
- Test every path-handling function with a path containing a space and a non-ASCII character.
- Executable names: append `.exe` on Windows through one helper.
