# lunaRun — Roadmap & Task List

Work top to bottom. Each **milestone** = one branch + one PR (`feat/m1.2-registry`), and is merged only when its
*Definition of Done* holds. Tick the boxes (`[x]`) in the same PR that completes a task.
Reference requirement IDs from [PRD.md](PRD.md) in commits/PRs.

**Global Definition of Done (every milestone):**
CI green · new `luna-core` logic has unit tests · no `clippy` warnings · UI strings go through i18n ·
works with a LUNA_HOME path containing a space · `CHANGELOG.md` "Unreleased" updated · docs updated if behaviour differs from them.

---

## Phase 0 — Foundation (`v0.0.x`)

Goal: an empty app that builds, tests and **releases installers for all 3 OSes from a tag**. Prove the pipeline before building features.

### M0.1 Scaffold
- [x] Cargo workspace with `crates/luna-core`; `rust-toolchain.toml` (Rust 1.99, edition 2024)
- [x] Tauri 2 + React 19 + TS (strict) + Vite + Tailwind 4 + shadcn/ui (Button only so far); pnpm; `src-tauri` in the Cargo workspace
- [x] App identifier `com.digitallybird.lunarun` (PRD Q2), product name `lunaRun` from a single constant (`luna-core`; `tauri.conf.json` is checked against it by a test)
- [ ] Choose router + TS type generation (`ts-rs` or `specta`); record both in DECISIONS.md
- [ ] ESLint, Prettier, `cargo fmt`, `clippy` configs; `.editorconfig`
- [ ] i18n setup (e.g. `i18next`), English strings file
- [ ] App shell: sidebar (Dashboard, Projects, Runtimes, Services, Settings, Diagnostics) with placeholder pages, light/dark/system theme
- [ ] `luna-core`: `paths.rs` (LUNA_HOME resolution + guarded `safe_remove`), `error.rs` (`LunaError` + codes), `state.rs` (atomic JSON + schema) with tests
- [ ] One end-to-end command `app_info()` → shows version, OS, arch, LUNA_HOME in Diagnostics

### M0.2 CI/CD
- [ ] `ci.yml` as in BUILD-AND-RELEASE §3
- [ ] `release.yml` (draft releases, tag/version check)
- [ ] `scripts/bump-version.ts` + `pnpm bump`
- [ ] Issue templates (bug, feature), PR template
- [ ] Dependabot for cargo, npm, github-actions
- [ ] **Tag `v0.0.1`** → all 3 installers build; install and launch on Windows + macOS manually

---

## Phase 1 — MVP (`v0.1.0`)

### M1.1 Download & extract engine
- [ ] Resumable HTTP download with progress callback, cancel, retry (FR-RT-02)
- [ ] SHA-256 verify (FR-RT-03, NFR-05)
- [ ] Extract zip / tar.gz / tar.xz with zip-slip guard and `stripComponents`
- [ ] Job system: job ids, dedupe, `job:progress` / `job:done` events, `job_cancel`
- [ ] Tests: checksum mismatch, traversal archive rejected, cancel mid-download, resume

### M1.2 Registry
- [ ] Registry types + parser (ignore unknown fields, schema check)
- [ ] Fetch → cache → bundled fallback; `LUNA_REGISTRY_URL` override
- [ ] `scripts/registry/generate.ts`: PHP (Windows releases.json + static builds), Node (index.json), Composer, MySQL, MariaDB, Caddy
- [ ] Bundle the generated `registry.json` as a fallback
- [ ] Decide registry hosting (RUNTIMES §5) and record it in DECISIONS.md

### M1.3 Runtime manager
- [ ] `RuntimeAdapter` trait + adapters: php, composer, node, mysql, mariadb, caddy
- [ ] Install flow incl. staging + atomic rename + receipt (FR-RT-13); incomplete-install cleanup on start
- [ ] PHP post-install: php.ini generation, CA bundle, conf.d override dir (SERVICES §4); macOS ad-hoc codesign
- [ ] Windows VC++ redist check + guidance (FR-RT-12)
- [ ] Uninstall with in-use / bound-projects checks; DB data kept unless asked (FR-RT-05/06)
- [ ] **UI: Runtimes page** — tabs per runtime, Installed / Available lists, install progress, EOL badges, license link

### M1.4 Supervisor & services
- [ ] Supervisor with state machine, ready checks, restart policy (SERVICES §2)
- [ ] Windows Job Objects; Unix process groups; PID files + adopt/kill on launch (FR-SV-07)
- [ ] Log capture, rotation, ring buffer, throttled events (FR-SV-05)
- [ ] PortAllocator + owner lookup per OS (FR-SV-03)
- [ ] Services: `php-fcgi@` (Win), `php-fpm@` (mac/Linux), `mysql@`, `mariadb@` (incl. first-run init), `caddy`
- [ ] Graceful shutdown ordering on quit
- [ ] **UI: Services page** — list, state badges, start/stop/restart, live log viewer, port conflict dialog

### M1.5 Constraint engine
- [ ] Composer constraint parser + matcher (PROJECT-DETECTION §6), exhaustive table tests
- [ ] npm-semver range support for Node
- [ ] Interval intersection + union

### M1.6 Detection & resolution
- [ ] `detect()` for PHP, extensions, Node, DB hints, framework, doc root (PROJECT-DETECTION §2–5)
- [ ] Data files: `laravel-php.json`, `php-extensions.json`
- [ ] `resolve()` (§7)
- [ ] All fixtures from §8 with golden `expected.json`; ≥ 95 % pass

### M1.7 Projects & sites
- [ ] Project store, add (picker + drag & drop), remove (never deletes files), rename, change doc root
- [ ] Bindings (line/exact), manual override with incompatibility warning (FR-PJ-04)
- [ ] Caddyfile generation, validate, reload; per-site port (FR-ST-01..03)
- [ ] Project start/stop = ensure DB + PHP pool + Caddy site
- [ ] Quick actions: browser, folder, editor, copy DB credentials (FR-PJ-09)
- [ ] Open terminal with project env (FR-PJ-08, SERVICES §5)
- [ ] "Update DB_PORT in .env" with diff preview + backup; "Create database" (PROJECT-DETECTION §4)
- [ ] **UI: Projects list + Project detail** (detection checklist with one-click fixes, bindings, site URL, actions)

### M1.8 App shell
- [ ] Tray / menu bar with menu (FR-AP-01/02); close-to-tray
- [ ] First-run wizard + "Laravel starter pack" (FR-AP-06)
- [ ] Settings page (FR-AP-03); launch at login
- [ ] Diagnostics page + "Copy report" (FR-AP-04)
- [ ] Dashboard: running services, projects quick start, recent errors

### M1.9 MVP hardening → `v0.1.0`
- [ ] Manual test matrix: fresh Windows 11 VM, macOS arm64, macOS x64 (if available), Ubuntu 22.04
- [ ] PRD §11 scenario: installer → Laravel 12 app in browser in < 5 min
- [ ] Two projects on different PHP versions running at the same time
- [ ] Coexistence test: XAMPP installed and running on the same Windows machine → clean conflict messages
- [ ] Footprint check (NFR-01/02)
- [ ] README: install instructions + unsigned-app workarounds + screenshots

---

## Phase 2 — `v0.2` → `v0.4`

- [ ] **M2.1** `.test` domains: hosts-file helper with elevation, `# BEGIN/END lunaRun` block (FR-ST-04)
- [ ] **M2.2** Local HTTPS via Caddy internal CA + trust flow (FR-ST-05)
- [ ] **M2.3** `luna` CLI (`crates/luna-cli`) + opt-in shims on PATH, clean uninstall (FR-SH-01/02)
- [ ] **M2.4** Auto-update + update channel setting (FR-AP-05); registry signature verification (NFR-05)
- [ ] **M2.5** PHP extension toggles UI (FR-RT-10); enable missing extensions from detection (FR-PJ-06)
- [ ] **M2.6** `.lunarun.json` read/write (FR-PJ-10); file watching + re-scan prompt (FR-PJ-11)
- [ ] **M2.7** New runtimes: PostgreSQL, Valkey, Mailpit; matching detection hints
- [ ] **M2.8** "Update to latest patch" per line (FR-RT-08); service autostart (FR-SV-12)
- [ ] **M2.9** Xdebug (Windows DLLs; investigate static builds, PRD Q5)
- [ ] **M2.10** Detection: Symfony, WordPress (FR-PJ-13)

## Phase 3 — `v0.5+`

- [ ] Workers: queue / schedule / vite as supervised services (FR-SV-11)
- [ ] New Laravel project wizard (FR-PJ-12)
- [ ] Nginx / Apache per site (FR-ST-06)
- [ ] Adminer one-click DB UI
- [ ] Bring-your-own runtimes (FR-RT-11); import from XAMPP/Laragon/Herd
- [ ] Move LUNA_HOME
- [ ] More stacks (plugin-style adapters)
