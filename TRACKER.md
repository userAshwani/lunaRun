# lunaRun — Project Tracker

> **Purpose:** the handoff note between chat sessions and models. Read this first, then
> [CLAUDE.md](CLAUDE.md). Update it at the end of **every** turn that changes anything.
> Detailed task checklists live in [docs/ROADMAP.md](docs/ROADMAP.md). This file is the short version plus context
> that is not derivable from the code.

**Last updated:** 2026-10-30 (Phase 0 done, Phase 1 starting) · **Repo:** https://github.com/userAshwani/lunaRun (private, branch `main`)

## 1. Where we are

| | |
|---|---|
| **Current phase** | Phase 0 is done except the owner's smoke test of the `v0.0.1` draft and the first automatic public release. **Phase 1 (MVP, `v0.1.0`) is under way (M1.1).** |
| **Current milestone** | M1.1 Download and extract engine (2 of 4 tasks done). M0.1 and M0.2 are complete. |
| **Next step** | **Finish the `v0.0.2` release (owner approved), then M1.1 task 3.** (1) If the tag is not pushed yet: wait for CI green on `chore(release): v0.0.2`, then `git tag -a v0.0.2 -m "lunaRun v0.0.2" && git push origin v0.0.2`. (2) Watch it: `gh run list --workflow release.yml`, `gh run watch <id> --exit-status`, `gh run view <id> --log-failed`. The `publish` job must succeed. Fix failures with new commits; if a re-run needs the tag moved or deleted, ask the owner first. (3) Verify the public result: `gh release view v0.0.2 --json isDraft,isPrerelease,assets` (not a draft, assets named `lunaRun-windows-x64-setup.exe`, `lunaRun-windows-x64.msi`, `lunaRun-macos-universal.dmg`, `lunaRun-macos-universal.app.tar.gz`, `lunaRun-linux-x64.AppImage/.deb/.rpm`), and that each README link answers with a redirect: `curl -sI https://github.com/userAshwani/lunaRun/releases/latest/download/lunaRun-windows-x64-setup.exe` should return `302`. Re-render the README check (badges should now show `v0.0.2`). (4) The owner approved removing the unused hidden `v0.0.1` draft and its tag, but only after v0.0.2 is verified: `gh release delete v0.0.1 --cleanup-tag --yes`. (5) Ask the owner to install the Windows `.exe` and open the app (still the pending smoke test). Then continue with **M1.1 task 3 (archive extraction)**. Spec: `crates/luna-core/src/extract.rs`; FR-RT-02..04, NFR-05; docs/RUNTIMES.md section 4. Extract `.zip`, `.tar.gz`, `.tar.xz` into a given directory (caller passes a fresh `runtimes/<rt>/.staging-<uuid>` and renames it) with `strip_components`. Safety is the point (zip-slip): reject absolute paths, `..`, Windows drive letters and UNC paths, backslash tricks, and symlinks or hardlinks resolving outside the destination; never write through an archive-created symlink; cap total uncompressed size, entry count and per-entry size (bombs; defaults generous enough for MySQL ~1 GB, configurable). Preserve Unix modes (`+x`), ignore ownership. Progress callback and `CancellationToken`, on `spawn_blocking`. Crates: check what is maintained today and builds on Windows, macOS and Linux with no extra system libraries (`zip`; `tar` + `flate2`; for `.xz` compare `liblzma`/`xz2` which compile C against a pure-Rust option such as `lzma-rust2`/`lzma-rs`; record the choice as D-016). Tests build their own malicious archives in code (traversal, absolute path, drive letter, symlink escape, bomb) and must prove each is rejected and that nothing was written outside the destination; also strip_components, modes, path with space and non-ASCII, cancel, truncated or corrupt archive (clean error, no panic). Mutation-check it as was done for the download engine (disable one guard at a time, a test must fail). No job system or IPC in this step. |
| **Released versions** | `v0.0.2` is being released automatically (see *Next step*). `v0.0.1` is an unpublished draft that will be deleted after v0.0.2 is verified. |
| **App state** | Runs. Light-theme dashboard shell. **Diagnostics page shows live data from Rust** (first working IPC command, `app_info`). Everything else is placeholder. |

**How the owner works:** sends just **"start work"** (often in a fresh chat) → resume from *Next step* above, do exactly that
step, then stop. Before committing, update this file (it lands in the same commit), then commit and push to `main` with
**no Claude/AI attribution**. See "Owner's standing workflow" in CLAUDE.md.

## 2. Done log (newest first)

| Commit | What |
|---|---|
| *(this commit)* | **Release prep for v0.0.2** (owner approved: first automatic public release): `pnpm bump 0.0.2`, CHANGELOG `## [0.0.2] - 2026-10-30`. Also fixed a misfiled changelog (post-0.0.1 entries had landed in the `0.0.1` section; the bump script's empty-Unreleased check caught it) and added the rule to CLAUDE.md. The tag `v0.0.2` is pushed after CI is green on this commit; see *Next step*. |
| `538bfb4` | **M0.3 public presence (owner request, repo is now public):** logo (`assets/logo.svg`, rendered `logo-1024.png`) and real app icons via `pnpm tauri icon` (Android/iOS sets removed), README rebuilt (centered logo, badges, **Download table with `releases/latest/download/<stable name>` links**, first-run notes, honest pre-alpha status), release workflow renamed **"Build & Release"** with manual-build defaults all platforms + artifacts and a new `publish` job (`scripts/finalize-release.mjs`: renames installers to stable names, publishes only if every build passed; 22 tests, dry-run checked against the real v0.0.1 draft), CI runs Windows/macOS tests on every push (D-015 supersedes D-013), docs for the public repo, secrets scan clean. **Verified on GitHub:** CI green including the new Windows and macOS test jobs; the manual *Manual build (all)* run passed on all three platforms and uploaded 7 artifacts (the earlier artifact-quota failure was a private-repo limit and is gone); the publish job is skipped on non-tag runs as designed. README rendered through GitHub's Markdown API and reviewed. Not yet exercised: the publish job on a real tag (v0.0.2 does that). |
| `05f40f4` | **M1.1 tasks 1 and 2: download engine** (`crates/luna-core/src/download.rs`): `Downloader::download` fetches into `LUNA_HOME/downloads/<name>.part`, resumes with HTTP `Range` (restarts when ignored or when the partial file is stale: 416), retries network errors and 5xx 3 times with backoff (not 4xx), cancellable (keeps the `.part`), HTTPS only including redirects, size cap (default 4 GiB), then verifies SHA-256 (mismatch deletes the file, `CHECKSUM_MISMATCH`) and renames atomically; an existing verified file is reused with no network. New `ErrorCode`s `NETWORK`, `CHECKSUM_MISMATCH`, `CANCELLED`, `INVALID_URL` (bindings regenerated, English strings added). 4 unit tests, 18 loopback integration tests (own raw-TCP test server, no new test crates) and 2 opt-in online HTTPS tests (`-- --ignored`, run once locally: passed). **Mutation-checked:** removing Range, the checksum check, the redirect policy or the 4xx rule each makes a test fail (this found and fixed a weak redirect test). D-014 records `reqwest` + rustls + OS trust store. |
| `d8c678e` | Docs only: record the verified `v0.0.1` release run, close M0.2, set M1.1 as the next step. |
| `0886046` + tag `v0.0.1` | **M0.2 task 6: first real release.** `pnpm bump 0.0.1` (commit `chore(release): v0.0.1`, CI green), annotated tag `v0.0.1` pushed. The Release run succeeded for all three platforms on the first try (Linux 9.4 min, macOS 7.4 min, Windows 12.2 min, about 115 billable minutes). Result: a **draft** release `lunaRun v0.0.1` (not published) with NSIS `lunaRun_0.0.1_x64-setup.exe` 1.63 MB, `.msi` 2.26 MB, `.dmg` 4.15 MB, `.app.tar.gz` 4.05 MB, `.deb` 2.04 MB, `.rpm` 2.05 MB, `.AppImage` 77 MB. macOS binary verified **universal** (x86_64 + arm64) from its Mach-O header. |
| `d7bfdfb` | Docs only: record the Dependabot verification and set the `v0.0.1` tag as the next step (needs owner approval). |
| `fb626d2` | **M0.2 task 5: `.github/dependabot.yml`, verified on GitHub**: weekly, grouped updates for `cargo` (Tauri family group + minor/patch group), `npm` (`@tauri-apps/*` group + minor/patch group) and `github-actions` for `/` and `/.github/actions/setup`; commit prefixes `chore(deps)` / `ci(deps)`. Validated with Ajv against schemastore's `dependabot-2.0` (a broken file is rejected). GitHub's three *Dependabot Updates* runs all succeeded: npm detected pnpm and parsed the lockfile (35 direct deps checked), cargo checked 9 crates and routed `tauri`/`tauri-build` into the `tauri` group, actions found both directories. Everything is already current, so **no PRs were opened yet**. |
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
| **M0.2 CI/CD** | ✅ done | `ci.yml`, `release.yml`, bump script, templates, Dependabot, `v0.0.1` draft. **Owner still to do:** install and launch the Windows and macOS installers (smoke test). |
| **M1.1** Download & extract | 🟡 in progress | **Done:** download + SHA-256. **Left:** archive extraction, job system (ids, dedupe, progress events, cancel) |
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
- **The repository is public** (since 2026-10-30): standard runners are free, release downloads are public, history is public (commit author email `codeashwani@gmail.com`, owner's choice). D-015 supersedes the private-repo cost control D-013, so CI runs the Windows and macOS tests on every push. Branch protection/rulesets are now available for free (Settings > Rules), not set up. Enabling *private vulnerability reporting* (Settings > Code security) is recommended because SECURITY.md points to it. Both are owner actions.
- **Resolved:** the artifact-quota failure was specific to the private repo. Since the repo became public, manual builds upload artifacts normally (verified: 7 artifacts).
- **Release pipeline verified end to end at `v0.0.1`** (tag push builds Windows, macOS and Linux and creates a draft). Still unproven: that the installers *install and launch* (owner smoke test), signing and notarisation (none yet: Windows SmartScreen warning, macOS Gatekeeper needs right-click > Open), and the auto-update path (Phase 2). The Windows `-setup.exe` is a 32-bit NSIS stub by design (PE machine `0x14c`), which installs the 64-bit app. Tag runs attach installers as release assets, so the artifact-quota problem below does not affect them.
- **Installer sizes (measured at `v0.0.1`):** Windows 1.6 MB (NSIS) / 2.3 MB (MSI), macOS 4.2 MB (dmg), Linux `.deb` 2 MB, but the Linux **AppImage is 77 MB** because it bundles WebKitGTK. PRD NFR-01 (20 MB) therefore applies to Windows and macOS only, and the PRD says so. Sizes will grow once real features land: re-measure at `v0.1.0`.
- GitHub warns that `ubuntu-latest` migrates to Ubuntu 26 from 2026-10-19. CI uses `ubuntu-latest` and passed. Release builds pin `ubuntu-22.04` on purpose (older glibc, wider compatibility). If CI breaks on Linux for no code reason, check the runner image.
- **Issue forms render is unconfirmed:** the two forms pass schemastore's official schema and GitHub parsed `config.yml` (its API returns the security contact link), but GitHub's APIs (`community/profile`, GraphQL `issueTemplates`) only report Markdown templates, so they show no issue template even though forms exist. **Owner check, 10 seconds:** open https://github.com/userAshwani/lunaRun/issues/new/choose and confirm "Bug report" and "Feature request" appear (and no blank-issue link). If not, the form YAML needs a fix.
- **HTTPS path is verified only by an opt-in test:** `cargo test -p luna-core --test download_online -- --ignored` (needs internet, hits apache.org, pinned SHA-256). It passed on Windows once. Run it after touching `download.rs`, TLS features or the HTTP client; CI does not run it (it must not depend on the internet). The loopback tests cover everything else with plain HTTP. Not tested at all: corporate proxies and TLS-inspecting firewalls (the app uses system proxy settings and the OS trust store, D-014), and macOS/Linux behaviour of the TLS verifier.
- **Download engine is not wired to anything yet:** no IPC command, no UI, no job/progress events (M1.1 task 4). Progress is a plain callback. Note that a resumed download of a file that changed on the server fails the checksum and is deleted, so the user retries from zero by design.
- **Dependabot works with the pnpm 12 lockfile** (verified from its run logs). Its updater image has pnpm 12.10.1 while `package.json` pins 12.16.1, so when the first real npm PR appears, check that its `pnpm-lock.yaml` diff is sane (the extra leading YAML document must survive). The log notice `Failed to parse GITHUB_REGISTRIES_PROXY` is harmless (no private registries). Dependabot runs also use Actions minutes on a private repo; weekly keeps that small. Enabling *Dependabot security updates* is a repo setting for the owner (Settings > Code security), not done.
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
