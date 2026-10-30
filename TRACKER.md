# lunaRun — Project Tracker

> **Purpose:** the handoff note between chat sessions and models. Read this first, then
> [CLAUDE.md](CLAUDE.md). Update it at the end of **every** turn that changes anything.
> Detailed task checklists live in [docs/ROADMAP.md](docs/ROADMAP.md). This file is the short version plus context
> that is not derivable from the code.

**Last updated:** 2026-10-30 (M0.2 in progress) · **Repo:** https://github.com/userAshwani/lunaRun (private, branch `main`)

## 1. Where we are

| | |
|---|---|
| **Current phase** | Phase 0, Foundation (`v0.0.x`) |
| **Current milestone** | M0.2 CI/CD (5 of 6 tasks done, Dependabot result pending). M0.1 Scaffold is complete. |
| **Next step** | **Check Dependabot's first run, then close M0.2 task 5.** Pushing `.github/dependabot.yml` makes GitHub start a dynamic workflow named *Dependabot Updates* (visible with `gh run list --workflow "Dependabot Updates"` or `gh workflow list`; it can take a few minutes to appear). Read its result with `gh run view <id> --log`. The question to answer: does the `npm` ecosystem cope with this repo's pnpm 12 lockfile (extra leading YAML document `packageManagerDependencies`)? Also confirm the `cargo` run finds the workspace and `github-actions` finds both directories. Expect Dependabot to open its first PRs immediately for anything outdated (grouped, so roughly 5 to 10); each runs CI on Ubuntu (about 2-4 min). **Do not merge or close them without the owner**, just list them (`gh pr list`) and tell the owner what they are. Record the outcome (works / which ecosystem fails and why) in Known issues. If the npm run fails on the lockfile, the options are: drop the `packageManager` pin, or keep it and accept manual npm updates, tell the owner and let them choose. Then the last M0.2 task is **6: tag `v0.0.1`**, which needs the owner's go-ahead because it triggers the macOS runner (×10 minutes) and creates a draft release: propose the exact steps (set CHANGELOG notes, `pnpm bump 0.0.1`, commit `chore(release): v0.0.1`, tag, push) and confirm before running them. |
| **Released versions** | none yet (first tag `v0.0.1` closes Phase 0) |
| **App state** | Runs. Light-theme dashboard shell. **Diagnostics page shows live data from Rust** (first working IPC command, `app_info`). Everything else is placeholder. |

**How the owner works:** sends just **"start work"** (often in a fresh chat) → resume from *Next step* above, do exactly that
step, then stop. Before committing, update this file (it lands in the same commit), then commit and push to `main` with
**no Claude/AI attribution**. See "Owner's standing workflow" in CLAUDE.md.

## 2. Done log (newest first)

| Commit | What |
|---|---|
| *(this commit)* | **M0.2 task 5: `.github/dependabot.yml`** pushed: weekly, grouped updates for `cargo` (Tauri family group + minor/patch group), `npm` (`@tauri-apps/*` group + minor/patch group) and `github-actions` for `/` and `/.github/actions/setup`; commit prefixes `chore(deps)` / `ci(deps)`. Validated with Ajv against schemastore's `dependabot-2.0` (broken file rejected). **Dependabot's first run is still to be checked**, see *Next step*. |
| `2c1de81` | Docs only: note that issue-form rendering needs a one-time manual check. |
| `ad694b5` | **M0.2 task 4: GitHub issue forms and PR template** (`.github/ISSUE_TEMPLATE/bug_report.yml`, `feature_request.yml`, `config.yml` with blank issues off and a security link, `.github/PULL_REQUEST_TEMPLATE.md`). Forms validated with Ajv against schemastore's `github-issue-forms` and `github-issue-config` schemas (and a deliberately broken form is rejected). CI now also skips pushes that only touch issue templates or `dependabot.yml`; CONTRIBUTING points to the forms and to SECURITY.md. |
| `c99ec35` | **M0.2 task 3: `pnpm bump <version>`** (`scripts/bump-version.mjs`): updates `package.json`, `tauri.conf.json`, Cargo `[workspace.package]`, `Cargo.lock` (`cargo update --workspace --offline`) and promotes the CHANGELOG Unreleased section; validates version (semver, no `v`, must increase unless `--force`), `--dry-run`, computes all edits first, restores files if cargo or the final check fails. 20 new tests (59 frontend/script tests total). Verified with real cargo on a throwaway copy: one changed line per file, lock changes only the two workspace crates. `check-versions.mjs` now exports `readVersions`. |
| `87d29f4` | Docs only: tick the release-workflow task, record the Linux verification, PRD NFR-01 note on the AppImage size. |
| `e1d58cf`, `9e3b74b` | **M0.2 task 2: release workflow, verified on Linux** (`.github/workflows/release.yml`): `plan` job runs `scripts/check-versions.mjs` (tag must equal the version in package.json, tauri.conf.json and Cargo.toml; 7 tests) and picks platforms; `build` matrix uses `tauri-apps/tauri-action@v1`; a tag push makes a draft release; manual runs (`platform` input, default linux; `keep_artifacts` off by default) only build and list the installers. `tauri.conf.json` bundle metadata and per-user NSIS install mode, `patchelf` in Linux deps. Manual run on `ubuntu-22.04` succeeded and produced `.deb` (2.1 MB), `.rpm` (2.1 MB), `.AppImage` (80.7 MB). `actionlint` clean, 39 frontend tests pass. **Windows and macOS builds are still unverified.** |
| `c620c18` | **M0.2 task 1: CI workflow, first run green** (`.github/workflows/ci.yml` + composite action `.github/actions/setup`): Lint (ESLint, Prettier, tsc + vite build, rustfmt, clippy), Test (36 Rust + 32 frontend tests, ts-rs drift check, symlink test now really runs on Linux) in 4m22s, debug app Build in 2m53s, all on Ubuntu; Windows/macOS matrix manual only (D-013, run `gh workflow run CI`). `packageManager` pinned to pnpm 12.16.1 (lockfile regenerated). `actionlint` clean. |
| `78aff18` | **First end-to-end IPC command** `app_info` + Diagnostics page (system card, copy report, loading/error states). `luna-core::diagnostics` (`AppInfo`, `Os`, `Arch`), `LunaHome::from_env`, Tauri command with least-privilege permission (`build.rs` `COMMANDS` + `allow-app-info`, generated `src-tauri/permissions/`), managed `LunaHome` state that creates the data folder at startup, `src/lib/ipc.ts` (`IpcError`), TanStack Query, Badge/Skeleton/ErrorState components, typed error strings per `ErrorCode`. 36 Rust tests + 32 frontend tests; verified in the real app window. CLAUDE.md: "start work" resume protocol and IPC-command checklist. **Closes M0.1.** |
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
| **M0.2 CI/CD** | 🟡 in progress | **Done:** `ci.yml`, `release.yml` (Linux verified), bump script, issue/PR templates, Dependabot config. **Left:** verify Dependabot run, tag `v0.0.1` (first real Windows/macOS/release test) |
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
- **Private repo:** release downloads and the auto-updater only work for signed-in collaborators until it is public. Actions minutes are limited (macOS ×10). So CI runs on Ubuntu only per push, skips docs-only pushes, and the Windows/macOS matrix is manual (`gh workflow run CI`), see D-013. Run the matrix by hand after changing OS-specific code, and switch it to run on push when the repo goes public.
- **GitHub artifact storage quota is full** on the account (`Failed to CreateArtifact: Artifact storage quota has been hit`), although this repo holds 0 artifacts, so usage comes from elsewhere on the account. Check https://github.com/settings/billing. Workaround in place: manual release runs do not upload artifacts unless `keep_artifacts=true`, and they print the installer list instead. Tag runs attach installers as release assets, which are not affected, but that path has **never run**.
- **Release path untested:** the draft-release creation on a tag, and the Windows and macOS builds (universal binary, NSIS per-user installer), have not been run. The first real test is M0.2's last task (tag `v0.0.1`), so budget for fix-up commits there. Expect macOS ×10 minutes.
- **Linux AppImage is 80.7 MB** (it bundles WebKitGTK), the `.deb` is 2.1 MB. PRD NFR-01 (≤ 20 MB installer) is therefore meant for Windows/macOS, see the PRD note. Re-measure the Windows/macOS installers at `v0.0.1`.
- GitHub warns that `ubuntu-latest` migrates to Ubuntu 26 from 2026-10-19. CI uses `ubuntu-latest` and passed. Release builds pin `ubuntu-22.04` on purpose (older glibc, wider compatibility). If CI breaks on Linux for no code reason, check the runner image.
- **Issue forms render is unconfirmed:** the two forms pass schemastore's official schema and GitHub parsed `config.yml` (its API returns the security contact link), but GitHub's APIs (`community/profile`, GraphQL `issueTemplates`) only report Markdown templates, so they show no issue template even though forms exist. **Owner check, 10 seconds:** open https://github.com/userAshwani/lunaRun/issues/new/choose and confirm "Bug report" and "Feature request" appear (and no blank-issue link). If not, the form YAML needs a fix.
- **Lockfile has an extra YAML document at the top** (`packageManagerDependencies`) because `package.json` now pins `packageManager`. That is pnpm 12's normal format. Do not delete it, or `pnpm install --frozen-lockfile` fails in CI.
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

**Workflow lint** (do this before pushing any `.github/workflows/*.yml`, runner minutes are limited): download actionlint into the
scratchpad with `gh release download --repo rhysd/actionlint --pattern "actionlint_*_windows_amd64.zip"`, unzip, and run
`actionlint.exe .github/workflows/<file>.yml` on workflow files only (it rejects `action.yml` composite actions when passed directly,
but follows local `uses: ./...` references). After pushing, follow the run with `gh run watch <id> --exit-status` and read failures with
`gh run view <id> --log-failed`. CI uses Node 24 LTS while this machine has Node 26, and both pass.

**Validating GitHub YAML configs** (issue forms, `config.yml`, `dependabot.yml`, which fail silently or with vague errors on GitHub):
in a scratchpad folder run `npm init -y && npm i ajv yaml`, download the schema from schemastore
(`https://json.schemastore.org/github-issue-forms.json`, `github-issue-config.json`, `dependabot-2.0.json`) into a `schemas/` subfolder, and
validate each parsed YAML file with `new Ajv({ strict: false, allErrors: true }).compile(schema)`. Compile each schema only once (Ajv rejects a
duplicate `$id`), and check that GitHub's extra rules hold, such as unique `id` and `label` inside an issue form. Also feed it one deliberately broken
file to prove the validator can fail. Run Prettier too: `pnpm lint` checks `.github/**/*.yml`, and CI's lint job fails on unformatted YAML.

## 8. Handoff checklist for a new session

1. Read this file, then CLAUDE.md. Do not re-read all docs unless the step needs them (the step's topic doc is named in ROADMAP).
2. `git pull`, `git log --oneline -5`, and check `git status` is clean.
3. Wait for "start work", then do **only** the step in section 1.
4. At the end: tick ROADMAP boxes, update CHANGELOG, update this file (sections 1, 2, 3), commit, push.
