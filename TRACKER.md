# lunaRun — Project Tracker

> **Purpose:** the handoff note between chat sessions and models. Read this first, then
> [CLAUDE.md](CLAUDE.md). Update it at the end of **every** turn that changes anything.
> Detailed task checklists live in [docs/ROADMAP.md](docs/ROADMAP.md). This file is the short version plus context
> that is not derivable from the code.

**Last updated:** 2026-10-30 (end of M0.1) · **Repo:** https://github.com/userAshwani/lunaRun (private, branch `main`)

## 1. Where we are

| | |
|---|---|
| **Current phase** | Phase 0, Foundation (`v0.0.x`) |
| **Current milestone** | M0.1 Scaffold is **complete**. Next is M0.2 CI/CD (0 of 6 tasks done). |
| **Next step** | **M0.2 task 1: `.github/workflows/ci.yml`** as specified in [docs/BUILD-AND-RELEASE.md](docs/BUILD-AND-RELEASE.md) §3. Jobs: `lint` (pnpm lint, `cargo fmt --check`, clippy `-D warnings`), `test` (pnpm test, `cargo test --workspace`), `build` (`pnpm tauri build --no-bundle`), all on ubuntu with rust-cache + pnpm cache and cancel-in-progress concurrency. Linux needs the Tauri apt packages (webkit2gtk-4.1 etc.). Add the **ts-rs drift check** from D-012 (after `cargo test`, run `git diff --exit-code src/bindings`). Windows + macOS `cargo test -p luna-core` jobs only on push to `main` (private-repo minutes: macOS counts ×10). After pushing, watch the run (`gh run watch`) and fix until green. Ubuntu will run the symlink test that Windows skips. |
| **Released versions** | none yet (first tag `v0.0.1` closes Phase 0) |
| **App state** | Runs. Light-theme dashboard shell. **Diagnostics page shows live data from Rust** (first working IPC command, `app_info`). Everything else is placeholder. |

**How the owner works:** sends just **"start work"** (often in a fresh chat) → resume from *Next step* above, do exactly that
step, then stop. Before committing, update this file (it lands in the same commit), then commit and push to `main` with
**no Claude/AI attribution**. See "Owner's standing workflow" in CLAUDE.md.

## 2. Done log (newest first)

| Commit | What |
|---|---|
| *(this commit)* | **First end-to-end IPC command** `app_info` + Diagnostics page (system card, copy report, loading/error states). `luna-core::diagnostics` (`AppInfo`, `Os`, `Arch`), `LunaHome::from_env`, Tauri command with least-privilege permission (`build.rs` `COMMANDS` + `allow-app-info`, generated `src-tauri/permissions/`), managed `LunaHome` state that creates the data folder at startup, `src/lib/ipc.ts` (`IpcError`), TanStack Query, Badge/Skeleton/ErrorState components, typed error strings per `ErrorCode`. 36 Rust tests + 32 frontend tests; verified in the real app window. CLAUDE.md: "start work" resume protocol and IPC-command checklist. **Closes M0.1.** |
| `31ec7ab` | `luna-core` foundation: `error.rs`, `paths.rs`, `state.rs` with 31 tests. ts-rs bindings `ErrorCode.ts` and `LunaError.ts`. |
| `4bd9ed7` | UI/UX redesign: light default theme, Inter font, design tokens, grouped sidebar, dashboard with stat cards and get-started card, shared components (Card, PageHeader, EmptyState, StatCard). CLAUDE.md: UI/UX standard + tracker rules. This TRACKER.md. |
| `9cac1d2` | App shell: sidebar nav, hash routing, six placeholder pages, theme switcher |
| `8c25690` | i18n (`i18next`, typed keys, English strings) |
| `38269f7` | ESLint, Prettier, Vitest, clippy/rustfmt configs; `pnpm lint/typecheck/test` |
| `df2fef1` | Decisions D-011 (React Router, hash) and D-012 (`ts-rs`); `.cargo/config.toml` |
| `2e2b2b6` | `PRODUCT_NAME` / `APP_IDENTIFIER` constants + config consistency test |
| `9fa6a9a` | Tauri 2 + React 19 + TS + Vite + Tailwind 4 + shadcn/ui scaffold; builds and launches |
| `dc925f1` | Cargo workspace, `luna-core` crate, pinned Rust toolchain |
| `734af96` | All planning docs (PRD, architecture, runtimes, detection, services, build/release, roadmap, decisions, repo setup), Apache-2.0 license |

## 3. Pending work

Full checklists with requirement IDs: [docs/ROADMAP.md](docs/ROADMAP.md).

| Milestone | Status | Summary |
|---|---|---|
| **M0.1 Scaffold** | ✅ done | |
| **M0.2 CI/CD** | ⬜ next | `ci.yml`, `release.yml`, `scripts/bump-version.ts`, issue/PR templates, Dependabot; **tag `v0.0.1`** and test installers on Windows + macOS |
| **M1.1** Download & extract | ⬜ | resumable downloads, SHA-256, zip-slip-safe extraction, job system |
| **M1.2** Registry | ⬜ | registry parser, fetch/cache/fallback, generator script |
| **M1.3** Runtime manager | ⬜ | adapters, install/uninstall, php.ini, VC++ check, Runtimes page |
| **M1.4** Supervisor & services | ⬜ | process supervision, ports, logs, Services page |
| **M1.5** Constraint engine | ⬜ | Composer + npm semver parsing/matching |
| **M1.6** Detection & resolution | ⬜ | the core feature, golden fixtures |
| **M1.7** Projects & sites | ⬜ | project store, Caddy sites, terminal, `.env` fixes, Projects page |
| **M1.8** App shell features | ⬜ | tray, first-run wizard, settings, diagnostics, dashboard with live data |
| **M1.9** MVP hardening → `v0.1.0` | ⬜ | manual test matrix, footprint, README screenshots |
| **Phase 2 / 3** | ⬜ | `.test` domains + HTTPS, `luna` CLI, auto-update, more runtimes; workers, wizard, Nginx/Apache |

## 4. Open decisions and questions

| # | Question | Default until decided | Needed by |
|---|---|---|---|
| Q1 | Final product name (lunaRun is a working name). Trademark/domain check. | keep `lunaRun` | before public |
| Q3 | Self-build static PHP for macOS/Linux, or use upstream static-php builds | upstream first | M1.2 |
| Q4 | Code signing budget (Apple Developer $99/yr, Windows signing) | unsigned while private | before public `v0.1.0` |
| Q5 | Xdebug with static PHP builds | investigate in Phase 2 | Phase 2 |
| — | Registry hosting while the app repo is private (needs a public URL, see RUNTIMES §5) | bundled fallback + `LUNA_REGISTRY_URL` | M1.2 |
| — | Git author email: machine uses `codeashwani@gmail.com`; project email is `hello@digitallybird.com` | leave as is | owner's call |

Decisions already made are in [docs/DECISIONS.md](docs/DECISIONS.md) (D-001 … D-012).

## 5. Known issues and caveats

- **First `pnpm tauri build` is slow** (about 8 min cold, about 4 min incremental, LTO). For UI work use `pnpm dev` + a headless browser instead.
- **Windows signing:** none yet. Installers will trigger SmartScreen until signed.
- **Private repo:** release downloads and the auto-updater only work for signed-in collaborators until it is public. Actions minutes are limited (macOS ×10).
- **Symlink test did not really run here:** `safe_remove_unlinks_symlinks_without_touching_the_target` returns early on this Windows account (creating symlinks needs admin). It will run for real on Linux/macOS once CI exists (M0.2). Do not assume Windows symlink behaviour is covered.
- `ErrorCode` currently holds only the codes used so far (`NOT_FOUND`, `PERMISSION_DENIED`, `IO`, `PATH_OUTSIDE_ROOT`, `INVALID_PATH`, `STATE_CORRUPT`, `STATE_SCHEMA_TOO_NEW`, `INTERNAL`). Add the ones from ARCHITECTURE §6 (`PORT_IN_USE`, `CHECKSUM_MISMATCH`, …) when their feature lands.
- `state::load` returns `Recovered` when a file is corrupt but does not write defaults back. Callers decide when to `save`. Surface `Recovered` / `Migrated` outcomes in Diagnostics (M1.8).
- **Running the app creates the data folder** `C:\Users\Lenovo\AppData\Local\lunaRun` (`runtimes`, `config`, `data`, `logs`, `run`, `cache`) on this machine. It is empty and safe to delete. Set `LUNA_HOME` to use another location (Diagnostics shows which one is active).
- **Adding an IPC command** needs the capability entry as well as the handler (checklist in CLAUDE.md "Commands"). Forgetting it fails at runtime only, and unit tests mock the backend, so check new commands in the real window.
- Diagnostics does not show free disk space or prerequisite checks yet (PRD FR-AP-04 lists more). Those arrive with M1.3/M1.8.
- Theme choice is stored in `localStorage` for now. It moves to `settings.json` when the settings backend exists.
- Dashboard counters are honest zeros until the backend exists. Replace with live queries in M1.3/M1.4/M1.7.
- One unexplained observation: the very first real-window screenshot opened on *Projects* instead of Dashboard. It did not reproduce in three later launches. Re-check if it appears again.

## 6. Environment notes (this machine)

- Windows 11, shell: PowerShell + Git Bash. Repo path contains spaces (`D:\DIGITALLY BIRD\2026\github\lunaRun`), which is useful as a built-in test of NFR-07.
- Installed by Claude on 2026-10-29: **Rust 1.99.1** (rustup, user profile), **pnpm** (npm global). Node v26 was already present.
- In a fresh terminal make sure `%USERPROFILE%\.cargo\bin` and `%APPDATA%\npm` are on `PATH`.
- Git remote `origin` is set up and authenticated through `gh` (account `userAshwani`). Token scopes: `repo`, `workflow`, `read:org`, `gist`, so pushing workflow files works.
- Symlinks cannot be created by this Windows account (needs admin or Developer Mode).

## 7. UI verification recipe

Real-window smoke test is slow, so for design work: run `pnpm exec vite --port 1420`, then drive Edge headless with
`playwright-core` (`executablePath` = `C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe`) from a **scratchpad**
folder (not the repo). Seed the theme with `localStorage.setItem("lunarun.theme", "dark" | "light")`, screenshot at 1400×900 and 900×600, then
*look at the images*. Stop the dev server afterwards.

**Real-window check** (needed whenever IPC, permissions or the Tauri config change, since unit tests mock the backend):
`pnpm tauri build --no-bundle`, then from PowerShell start `target\release\lunarun.exe`, make the process DPI-aware
(`SetProcessDPIAware`), find the nav link with UI Automation (`AutomationElement.FromHandle`, a `Hyperlink` named e.g.
"Diagnostics") and call its `InvokePattern.Invoke()` (a synthetic mouse click did not register), then capture the window with
`PrintWindow(hwnd, hdc, 2)` into a PNG in the scratchpad and look at it.

## 8. Handoff checklist for a new session

1. Read this file, then CLAUDE.md. Do not re-read all docs unless the step needs them (the step's topic doc is named in ROADMAP).
2. `git pull`, `git log --oneline -5`, and check `git status` is clean.
3. Wait for "start work", then do **only** the step in section 1.
4. At the end: tick ROADMAP boxes, update CHANGELOG, update this file (sections 1, 2, 3), commit, push.
