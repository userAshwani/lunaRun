# lunaRun — Product Requirements Document (PRD)

| | |
|---|---|
| **Product** | lunaRun (working name) |
| **Owner** | Digitally Bird |
| **Doc status** | v1.0 — approved for Phase 0/1 implementation |
| **Last updated** | 2026-10-29 |
| **Related** | [ARCHITECTURE](ARCHITECTURE.md) · [RUNTIMES](RUNTIMES.md) · [PROJECT-DETECTION](PROJECT-DETECTION.md) · [SERVICES](SERVICES.md) · [ROADMAP](ROADMAP.md) |

Requirement keywords **MUST / SHOULD / MAY** follow RFC 2119.
Every requirement has an ID (`FR-…`, `NFR-…`). Code, tests and PRs SHOULD reference these IDs.

---

## 1. Summary

lunaRun is a desktop app (Windows, macOS, Linux) that manages a local PHP/Laravel development
environment. Unlike XAMPP/WAMP, it can install **many versions of each runtime side by side**, run
**several of them at the same time**, and **inspect a project folder** to recommend the correct
versions for that project.

The first release focuses on **PHP + Laravel**. The architecture must allow other stacks later
without a rewrite.

## 2. Problem

1. XAMPP/WAMP/MAMP ship **one fixed version** of PHP and MySQL. Switching means reinstalling or
   hand-editing configs and `PATH`.
2. Developers with several client projects need **different PHP/DB/Node versions per project** at the
   same time.
3. Picking the right version is manual: you read `composer.json`, guess, and then hit
   `Your PHP version does not satisfy that requirement` errors.
4. Good tools exist (Laravel Herd, ServBay, Laragon), but they are closed source, paid for key
   features, or single-platform.

## 3. Target users

| Persona | Description | Key need |
|---|---|---|
| **Freelancer / agency dev** (primary) | Maintains 5–30 client projects on mixed Laravel/PHP versions | Switch per project instantly, no conflicts |
| **Student / beginner** | Learning Laravel, used XAMPP before | One installer, one click, works |
| **Team lead** | Wants every dev on the same versions | Project-declared versions picked up automatically |
| **Legacy maintainer** | Old PHP 7.4 / MySQL 5.7 apps next to new ones | Old and new versions installed together |

## 4. Goals and non-goals

### Goals (v1.x)
- G1. Install, keep and remove **multiple versions** of PHP, Composer, Node.js, MySQL, MariaDB and the web server.
- G2. Run **several versions at the same time** (for example two PHP versions and two DB servers).
- G3. **Project-aware**: add a folder, then detect requirements, recommend versions and fix them in one click.
- G4. **One codebase**, one release pipeline: GitHub Actions builds the installers for every OS from a version tag.
- G5. Free and open source under a permissive license.
- G6. Never break the user's system: no admin rights for normal use, no global `PATH` hijacking without consent, no edits to project files without consent.

### Non-goals (for now)
- N1. Production hosting or deployment.
- N2. Replacing Docker / Laravel Sail / DDEV. lunaRun runs native processes, not containers.
- N3. Remote servers, SSH, cloud sync.
- N4. Non-PHP stacks (Python, Ruby, Go) in v1. The architecture should *allow* them later.
- N5. Mobile apps.

## 5. Competitive landscape (context only)

| Tool | Multi-version side by side | Project detection | Cross-platform | Open source |
|---|---|---|---|---|
| XAMPP / WAMP / MAMP | ✗ / partial / ✗ | ✗ | partial | partial |
| Laragon | partial | ✗ | Windows only | ✗ (recent versions) |
| Laravel Herd | ✓ (PHP) | partial (`herd isolate`) | Win + mac | ✗ |
| ServBay | ✓ | ✗ | mac + Win | ✗ |
| FlyEnv | ✓ | ✗ | ✓ | ✓ |
| **lunaRun** | ✓ | ✓ (core feature) | ✓ | ✓ |

Our edge: **project-aware version resolution** + truly side-by-side runtimes + open source + cross-platform.
Do **not** copy UI, names, assets or code from these products.

## 6. Core concepts (glossary)

| Term | Meaning |
|---|---|
| **Runtime** | A kind of software lunaRun manages: `php`, `composer`, `node`, `mysql`, `mariadb`, `caddy`, … |
| **Runtime version** | One installed copy, e.g. `php 8.3.14`. Stored in its own folder. Many can coexist. |
| **Version line** | Major.minor family, e.g. `php 8.3`. A project may bind to a *line* ("latest 8.3 installed") or an *exact* version. |
| **Service** | A long-running process started from a runtime version: a PHP FastCGI pool, a DB server, the web server. |
| **Project** | A folder on disk the user added to lunaRun. |
| **Binding** | The runtime versions a project uses (php, node, database, …). |
| **Site** | A project served over HTTP by the web server (MVP: `http://localhost:<port>`; later `https://<name>.test`). |
| **Registry** | A signed JSON manifest listing downloadable versions with URLs + SHA-256 per OS/arch. See [RUNTIMES](RUNTIMES.md). |
| **Detection report** | What lunaRun found in a project (constraints, sources, recommended versions, warnings). |
| **LUNA_HOME** | lunaRun's data directory (runtimes, configs, DB data, logs). |

## 7. Functional requirements

### 7.1 Runtime manager

| ID | Requirement | Phase |
|---|---|---|
| FR-RT-01 | The app MUST show, per runtime, the **available** versions (from the registry) and the **installed** versions. | 1 |
| FR-RT-02 | The user MUST be able to install any available version. Installs run in the background with progress (bytes, %, speed) and can be cancelled. | 1 |
| FR-RT-03 | Downloads MUST be verified with SHA-256 before extraction. A mismatch aborts the install and deletes the file. | 1 |
| FR-RT-04 | Multiple versions of the same runtime (including several patch versions of one line) MUST be installable at once. | 1 |
| FR-RT-05 | The user MUST be able to uninstall a version. If it is running or bound to a project, show a warning listing the affected projects. | 1 |
| FR-RT-06 | Uninstalling a DB server MUST NOT delete its data directory unless the user explicitly ticks "also delete databases". | 1 |
| FR-RT-07 | Versions past end-of-life SHOULD show an "EOL" badge (data from the registry). | 1 |
| FR-RT-08 | The app SHOULD offer an "Update to latest patch" action per line (e.g. 8.3.12 → 8.3.14) that moves line-bound projects over automatically. | 2 |
| FR-RT-09 | Each PHP version MUST get its own `php.ini`, generated from a template with Laravel-friendly defaults. | 1 |
| FR-RT-10 | Per PHP version, the user SHOULD be able to toggle extensions (bundled ones) from the UI. | 2 |
| FR-RT-11 | The user MAY register an **external** runtime ("bring your own binary", e.g. an existing XAMPP/Homebrew PHP). It is shown as read-only/unmanaged. | 3 |
| FR-RT-12 | On Windows, before the first PHP install, the app MUST check for the Microsoft Visual C++ 2015–2022 x64 redistributable and guide the user to install it if it is missing. | 1 |
| FR-RT-13 | Installs MUST be atomic: a failed or cancelled install leaves no half-installed version visible. | 1 |

**MVP runtimes:** PHP, Composer, Node.js, MySQL, MariaDB (Windows/Linux; macOS see [RUNTIMES](RUNTIMES.md)), Caddy (web server).
**Later:** PostgreSQL, Redis-compatible (Valkey), Mailpit, Nginx, Apache httpd, Meilisearch, MinIO, Xdebug.

### 7.2 Services

| ID | Requirement | Phase |
|---|---|---|
| FR-SV-01 | The user MUST be able to start/stop/restart each service, and use "Start all / Stop all" for a project. | 1 |
| FR-SV-02 | Several versions of the same runtime MUST be able to run at the same time on different ports (e.g. MySQL 8.0 on 3307 and 8.4 on 3306). | 1 |
| FR-SV-03 | Before starting, the app MUST check the port. If it is taken, show which process holds it (name + path where possible, e.g. XAMPP's `mysqld.exe`) and offer another port. | 1 |
| FR-SV-04 | Service state (stopped / starting / running / stopping / crashed / error) MUST update live in the UI. | 1 |
| FR-SV-05 | stdout/stderr and the service's own log files MUST be viewable live in the app, with rotation (5 MB × 3 files). | 1 |
| FR-SV-06 | Crashed services SHOULD auto-restart up to 3 times within 60 s, then stay in `crashed` and notify. | 1 |
| FR-SV-07 | Quitting the app MUST stop all services gracefully (DBs use a clean shutdown). If the app crashes, no orphan process may stay unnoticed: on next launch the app MUST detect leftovers and offer to adopt or kill them. | 1 |
| FR-SV-08 | DB servers MUST bind to `127.0.0.1` only by default. | 1 |
| FR-SV-09 | DB data directories MUST be separate per engine and major.minor line (data from 8.0 and 8.4 never share a folder). | 1 |
| FR-SV-10 | A new DB data dir is initialised automatically on first start (root user, empty password — local dev only, clearly shown in the UI). | 1 |
| FR-SV-11 | The app SHOULD supervise per-project workers (`php artisan queue:work`, `schedule:work`, `npm run dev`) as services. | 3 |
| FR-SV-12 | Services MAY auto-start on app launch (per project setting). | 2 |

### 7.3 Projects and detection (core feature)

| ID | Requirement | Phase |
|---|---|---|
| FR-PJ-01 | The user MUST be able to add a project by choosing a folder or dropping it on the window. | 1 |
| FR-PJ-02 | On add (and on demand via "Re-scan"), the app MUST run detection and show a **detection report**: framework + version, document root, PHP constraint and its sources, required PHP extensions, Node constraint, DB engine from `.env`. | 1 |
| FR-PJ-03 | For each runtime, the app MUST show: compatible **installed** versions, the **recommended** one, and compatible **downloadable** versions with a one-click "Install & use". | 1 |
| FR-PJ-04 | The user MUST be able to override any binding manually, including choosing an incompatible version (with a warning). | 1 |
| FR-PJ-05 | Detection MUST be read-only. lunaRun MUST NOT write into the project folder unless the user explicitly triggers an action that says so (e.g. "Write `.lunarun.json`", "Update DB_PORT in .env" with a diff preview). | 1 |
| FR-PJ-06 | Detection MUST warn about missing PHP extensions required by `composer.lock` and offer to enable them. | 1 (warn) / 2 (enable) |
| FR-PJ-07 | Detection rules and the Laravel→PHP table MUST be data-driven (JSON) so they can be updated without code changes. | 1 |
| FR-PJ-08 | "Open terminal here" MUST open the OS terminal in the project folder with the bound `php`, `composer`, `node`, `npm`, `npx` first on `PATH` (no global changes). | 1 |
| FR-PJ-09 | Quick actions: open in browser, open folder, open in editor (VS Code if found), copy DB credentials. | 1 |
| FR-PJ-10 | `.lunarun.json` in the project root (optional, user-committed) MUST be honoured as the strongest project-declared source, so teams can share bindings. | 2 |
| FR-PJ-11 | The app SHOULD watch `composer.json`, `composer.lock`, `package.json`, `.nvmrc`, `.env` and offer a re-scan when they change. | 2 |
| FR-PJ-12 | "New Laravel project" wizard (runs `composer create-project` / `laravel new` with chosen versions). | 3 |
| FR-PJ-13 | Detection MUST also recognise non-Laravel PHP projects: Symfony, WordPress, generic PHP (document root heuristics). | 1 (generic), 2 (others) |

Full rules: [PROJECT-DETECTION.md](PROJECT-DETECTION.md).

### 7.4 Sites / web server

| ID | Requirement | Phase |
|---|---|---|
| FR-ST-01 | Each project MUST get a stable port and be served at `http://localhost:<port>` (ports allocated from 8100 up and remembered). | 1 |
| FR-ST-02 | The web server MUST route each site to the FastCGI pool of the site's bound PHP version, so sites on different PHP versions run at the same time. | 1 |
| FR-ST-03 | Changing a project's PHP version MUST take effect with a reload, without restarting other sites. | 1 |
| FR-ST-04 | Sites SHOULD be reachable at `http://<name>.test` (hosts file entry, added with a one-time elevation prompt). | 2 |
| FR-ST-05 | Sites SHOULD support local HTTPS with a locally trusted CA (one-time trust prompt). | 2 |
| FR-ST-06 | The user MAY choose Nginx or Apache instead of Caddy per site. | 3 |

### 7.5 Shell integration

| ID | Requirement | Phase |
|---|---|---|
| FR-SH-01 | A `luna` CLI SHOULD be available (`luna use php 8.3`, `luna status`, `luna start`, `luna php -v`). | 2 |
| FR-SH-02 | Optional shims for `php`, `composer`, `node`, `npm`, `npx` that resolve the version from the current directory's project binding, falling back to a global default. Installing them adds one directory to the user `PATH`, only after explicit opt-in, and can be cleanly reverted. | 2 |

### 7.6 App shell, settings, updates

| ID | Requirement | Phase |
|---|---|---|
| FR-AP-01 | The app MUST live in the system tray / menu bar. Closing the window keeps services running; "Quit" stops them. | 1 |
| FR-AP-02 | Tray menu: status, Start/Stop all, list of projects (open in browser / terminal), Open lunaRun, Quit. | 1 |
| FR-AP-03 | Settings: LUNA_HOME location (on first run only, moving later is Phase 3), default versions per runtime, port ranges, theme (light/dark/system), launch at login, registry URL (advanced). | 1 |
| FR-AP-04 | **Diagnostics** page: OS, arch, LUNA_HOME, free disk, port check results, VC++ redist status, recent errors; "Copy report" button for bug reports (no secrets). | 1 |
| FR-AP-05 | In-app auto-update via signed update manifests on GitHub Releases. | 2 |
| FR-AP-06 | First-run wizard: pick LUNA_HOME, check prerequisites, offer a "Laravel starter pack" (latest PHP + Composer + Node LTS + MySQL LTS). | 1 |
| FR-AP-07 | UI language: English. Strings go through an i18n layer from day one so translations can be added. | 1 |

## 8. Non-functional requirements

| ID | Requirement |
|---|---|
| NFR-01 **Footprint** | Windows and macOS installer ≤ 20 MB (the Linux AppImage bundles WebKitGTK and is about 80 MB, the `.deb` is about 2 MB); idle app RAM ≤ 150 MB with the window closed (excluding services). |
| NFR-02 **Startup** | Window interactive in < 2 s on a mid-range laptop. |
| NFR-03 **No admin** | Normal use MUST NOT need admin/root. Elevation only for explicit actions (hosts file, trusting a CA, ports < 1024 on Linux), each with a prompt explaining why. |
| NFR-04 **Safety** | lunaRun MUST NOT delete or modify anything outside LUNA_HOME, except explicit user-confirmed actions. All destructive operations go through one guarded helper that refuses paths outside allowed roots. |
| NFR-05 **Integrity** | Every downloaded artifact is SHA-256 verified. The registry is fetched over HTTPS and (Phase 2) signature-verified with an embedded public key. Archive extraction is protected against path traversal (zip-slip). |
| NFR-06 **Privacy** | No telemetry. No network calls except registry fetch, runtime downloads and update checks, each of which can be switched off. `.env` values are never logged or sent anywhere. |
| NFR-07 **Paths** | Works with paths containing spaces and non-ASCII characters (e.g. `D:\DIGITALLY BIRD\…`, `C:\Users\José\…`). This is a mandatory test case. |
| NFR-08 **Reliability** | State files are written atomically (write temp → fsync → rename). Corrupt state files are backed up and recreated, never cause a crash loop. |
| NFR-09 **Offline** | Installed runtimes, services and projects MUST work fully offline. The last-fetched registry is cached. |
| NFR-10 **Accessibility** | Keyboard navigable, visible focus, WCAG AA contrast in both themes. |
| NFR-11 **Testability** | Detection, constraint resolution, registry parsing, config rendering and port allocation live in a UI-free core crate with unit tests (target ≥ 80% line coverage for that crate). |

## 9. Platform support matrix (v1)

| | Windows 10/11 x64 | macOS 12+ arm64 | macOS 12+ x64 | Linux x64 (glibc ≥ 2.31) |
|---|---|---|---|---|
| App | ✓ | ✓ (universal build) | ✓ | best effort |
| PHP | 7.4 – 8.5 (official builds) | 8.1+ (static builds) | 8.1+ | 8.1+ |
| Composer | ✓ | ✓ | ✓ | ✓ |
| Node.js | ✓ | ✓ | ✓ | ✓ |
| MySQL | 8.0, 8.4 LTS, 9.x | 8.0, 8.4, 9.x | 8.0, 8.4, 9.x | 8.0, 8.4, 9.x |
| MariaDB | ✓ | ✗ in v1 (no official macOS binaries) | ✗ in v1 | ✓ |
| Caddy | ✓ | ✓ | ✓ | ✓ |

Windows on ARM64: not in v1 (x64 emulation may work, not supported).

## 10. Release plan (summary)

| Phase | Version | Theme |
|---|---|---|
| 0 | 0.0.x | Foundation: repo, scaffold, CI, **release pipeline proven end-to-end** with an empty app |
| 1 | 0.1.0 (MVP) | Runtime manager, services, projects + detection, localhost sites, tray, terminal |
| 2 | 0.2 – 0.4 | `.test` domains + HTTPS, `luna` CLI + shims, auto-update, signed registry, extensions UI, PostgreSQL, Valkey, Mailpit |
| 3 | 0.5+ | Workers, new-project wizard, Nginx/Apache option, Adminer, BYO runtimes, more stacks |

Detail and task lists: [ROADMAP.md](ROADMAP.md).

## 11. Success metrics

- MVP: a fresh Windows and macOS machine goes from installer → Laravel 12 app in the browser on its detected PHP version in **under 5 minutes**, with no manual config editing.
- Two Laravel projects needing different PHP versions run **at the same time** in the browser.
- Detection recommends a correct PHP version for **≥ 95 %** of the fixture projects (see [PROJECT-DETECTION](PROJECT-DETECTION.md#test-fixtures)).
- Once public: GitHub stars, number of issues opened by non-maintainers, release download counts.

## 12. Open questions

Track decisions in [DECISIONS.md](DECISIONS.md) once resolved.

| # | Question | Default until decided |
|---|---|---|
| Q1 | Final product name (lunaRun is a working name). Check trademark/domain before going public. | Keep `lunaRun`; keep the name in one constant/config so a rename is cheap. |
| Q2 | Bundle identifier (permanent: changing it breaks updates and data paths). | `com.digitallybird.lunarun` |
| Q3 | Self-build macOS/Linux PHP in a `lunarun-binaries` repo vs. use upstream static-php builds directly. | Start with upstream static-php builds, move to self-built if coverage/extensions are insufficient. |
| Q4 | Code signing budget (Apple Developer $99/yr; Windows signing cert / Azure Trusted Signing). | Unsigned during private phase; decide before public v0.1.0. |
| Q5 | Xdebug on macOS/Linux static PHP builds (static builds cannot load shared extensions). | Investigate in Phase 2. |
