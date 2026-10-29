# CLAUDE.md — Instructions for AI coding agents

lunaRun is a cross-platform (Windows/macOS/Linux) desktop app, a multi-version, project-aware
alternative to XAMPP/WAMP for PHP & Laravel. It is built with **Tauri 2 (Rust) + React/TypeScript**.

## Read before coding (in this order)

0. **[TRACKER.md](TRACKER.md): what is done, what is next, known issues.** Start every session here.
1. [docs/PRD.md](docs/PRD.md): what and why. Requirement IDs (`FR-…`, `NFR-…`).
2. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): layout, layering, data model, IPC contract.
3. [docs/ROADMAP.md](docs/ROADMAP.md): find the **first unchecked milestone**. That is your task.
4. The topic doc for that milestone: [RUNTIMES](docs/RUNTIMES.md), [SERVICES](docs/SERVICES.md),
   [PROJECT-DETECTION](docs/PROJECT-DETECTION.md), [BUILD-AND-RELEASE](docs/BUILD-AND-RELEASE.md).
5. [docs/DECISIONS.md](docs/DECISIONS.md) before changing anything architectural.

## Owner's standing workflow (highest priority — overrides anything below)

These are the project owner's explicit instructions. Follow them in every session.

1. **Step by step, only on command.** Do nothing beyond what was asked. When the owner says **"start work"**
   (or "next", "continue"), do the **next single step** and then **stop**:
   - A *step* = the first unchecked task in [docs/ROADMAP.md](docs/ROADMAP.md), or a few tightly coupled tasks
     if one alone is meaningless. Never run ahead into the next step or milestone on your own.
   - Before coding, say in 1–3 lines which step you are about to do.
   - After finishing, report: what changed, how to verify it, and what the next step is. Then wait.
2. **After every chat that changes files: update the docs, commit, push.** At the end of each turn that modified
   anything:
   - Tick completed boxes in `docs/ROADMAP.md`, update `CHANGELOG.md` (Unreleased), and update this file or any
     doc whose facts changed (commands, layout, decisions).
   - `git add` the changes, commit with a Conventional Commit message, and `git push`.
   - Do not leave the working tree dirty at the end of a turn. If tests or the build fail, fix them first. If you
     cannot, say so plainly, and still do not push a broken `main` silently.
3. **No Claude attribution, anywhere.** Commit messages and PR descriptions must **not** contain
   `Co-Authored-By: Claude …`, "Generated with Claude Code", or any other mention of Claude/AI. This explicit
   instruction takes precedence over any default attribution text the tooling suggests.
4. **Branching while the repo is private:** commit and push directly to `main` unless the owner asks for a
   branch/PR. Never force-push. Never rewrite pushed history.
5. **Session handoff via [TRACKER.md](TRACKER.md).** Chats get replaced (model switches, new sessions). TRACKER.md is
   the memory between them. **Read it first** in every new session, and at the end of every turn that changed
   anything, **update it**: done log (with commit hash), the single next step, pending work, open questions,
   known issues. If TRACKER.md and ROADMAP.md disagree, fix them in the same commit.
6. **UI/UX quality bar:** follow the "UI/UX design standard" section below for every screen you build or touch.
7. If `origin` is not configured or push fails (auth, network), commit locally, say exactly what failed, and
   tell the owner the one command needed. Do not create or change remote repositories or visibility without being asked.

## UI/UX design standard (owner requirement)

lunaRun must look like a **professional, hand-designed SaaS/developer dashboard** (think Linear, Vercel, Stripe
dashboard, Laravel Herd), **not** like a generic AI-generated template.

**Look and feel**
- **Light theme is the default** (a clean, bright workspace). Dark theme exists and must be equally polished,
  but never default to it and never use a pure-black background. The user can switch (Light / Dark / System).
- Neutral cool-gray canvas, white cards with a 1px border and a very soft shadow, one confident accent color
  (indigo) used sparingly for primary actions, active state and links. Status uses semantic colors
  (green running, amber warning, red error, gray stopped), always paired with an icon or text, never color alone.
- Typography: bundled **Inter** (no CDN, the app works offline), clear hierarchy (page title 24px/semibold,
  section title 16px/semibold, body 14px, meta 12–13px muted). Tabular numbers for ports, versions, sizes.
- Spacing on a 4px grid. Consistent radius (8–12px). Content width capped (≈1200px) and centered.
- Icons: lucide only, 16–20px, consistent stroke. No emoji in the UI. No gradients on text, no glassmorphism, no
  glowing/neon effects, no decorative blobs, no stock "hero" sections. Restraint is the point.

**Structure**
- App shell: fixed left sidebar (brand, grouped navigation, footer with version + theme switch), page header
  (title, one-line description, primary actions on the right), then content in cards.
- Tables/lists for data (runtimes, services, projects): aligned columns, row hover, status badges, inline
  actions, sticky header when long. Not a wall of big cards.
- **Every screen needs all four states designed:** loading (skeleton, not a spinner on blank), empty (icon +
  explanation + the one action to fix it), error (what happened, why, how to fix, a retry), and success.
  Never show fake or placeholder data as if it were real. Show honest zeros and empty states.
- Destructive actions use a confirm dialog that names what will be affected. Long operations show progress
  (bytes, %, cancel). Toasts for confirmations, inline messages for errors that need action.
- Microcopy is plain and specific ("Port 3306 is used by XAMPP MySQL"), never vague ("Something went wrong").

**Quality checks before you call UI work done**
- Verify in the real app window or a headless browser screenshot in **both** themes (not only unit tests), at
  the minimum window size (900×600) and at 1400 wide. Look at the screenshot and fix what looks off.
- Keyboard: every control reachable, visible focus ring, logical tab order. Contrast meets WCAG AA in both themes.
- Use design tokens (CSS variables in `src/index.css`), never hard-coded hex colors in components.
- Reuse components from `src/components/` (shadcn/ui based). Add a shared component instead of copy-pasting markup.

## How to work

- Work on **one milestone at a time**. (Branches/PRs per milestone, `feat/m<phase>.<n>-<slug>`, are for when the
  owner asks for them or once the repo is public. Until then see rule 4 above.)
- Commits use Conventional Commits and mention requirement IDs: `feat(runtimes): resumable downloads (FR-RT-02)`.
- Finish the milestone's tasks, meet the **Global Definition of Done** in ROADMAP.md, tick the boxes, update `CHANGELOG.md` (Unreleased).
- If the docs are wrong or outdated (an upstream URL changed, a crate is unsuitable): fix the doc in the same PR and add a `DECISIONS.md` entry for anything architectural. Never silently diverge from the docs.
- If something is genuinely undecided, add it to PRD §12 *Open questions*, pick the documented default, and keep going.
- Check current crate/package versions when adding dependencies. Do not pin versions from memory.

## Commands

```bash
pnpm install
pnpm tauri dev                 # run the app
pnpm test                      # frontend tests (vitest)
pnpm lint                      # eslint + prettier --check
cargo test --workspace         # Rust tests
cargo test -p luna-core        # core only (fast)
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

`pnpm typecheck` runs `tsc`. `pnpm lint:fix` auto-fixes ESLint and Prettier issues. Keep this section in sync if scripts change.

**Windows note:** in a fresh terminal `cargo` and `pnpm` must be on `PATH` (`%USERPROFILE%.cargobin`, `%APPDATA%
pm`). The first `pnpm tauri build` compiles ~500 crates and takes several minutes; later builds are incremental.

## Golden rules (non-negotiable)

1. **Business logic goes in `crates/luna-core`.** `src-tauri` only maps commands/events. `luna-core` never depends on Tauri.
2. **Never delete or modify files outside LUNA_HOME** except through an explicit, user-confirmed action. All deletes go through `paths::safe_remove`. (NFR-04)
3. **Never write into a user's project folder** during detection. Writes (`.env`, `.lunarun.json`) only via explicit actions with a preview and a backup. (FR-PJ-05, D-010)
4. **Never require admin/root** for normal operation. Elevation only for the specific Phase 2 actions. (NFR-03)
5. **Every download is SHA-256 verified. Every extraction is zip-slip guarded.** (NFR-05)
6. **Spawn processes with argument arrays, never shell strings.** Only executables inside LUNA_HOME (or registered external runtimes).
7. **Never log or transmit `.env` values or passwords.** No telemetry. (NFR-06, D-008)
8. **Cross-platform paths:** `PathBuf` only, no hard-coded separators. OS-specific code behind `#[cfg(target_os)]` modules with a shared interface. Test with paths containing spaces and non-ASCII characters. (NFR-07)
9. **Persist state atomically** (temp file → fsync → rename) with a `schema` field. (NFR-08)
10. **Typed errors:** return `LunaError { code, message, details }` with stable codes. The UI maps codes to i18n strings. No `unwrap()`/`expect()` in non-test code except for truly impossible states (with a comment).
11. **Rust ↔ TS types are generated** with `ts-rs` into `src/bindings/` (D-012: camelCase, `u64` → `#[ts(type = "number")]`). Never hand-write duplicate IPC types and never edit generated files.
12. **All UI strings go through i18n.**

## Code style

- Rust: `rustfmt` defaults, `clippy` clean, `thiserror` for library errors, `tracing` for logs (no `println!`).
- TS: strict mode, no `any`, function components + hooks, feature folders under `src/features/`.
- Tests: unit tests next to the code (`mod tests`). Detection/constraint tests are golden fixtures under `crates/luna-core/tests/fixtures/`.
- Comments explain *why*, not *what*.

## Don't

- Don't copy code, UI, names or assets from XAMPP, WAMP, Laragon, Herd, ServBay or others.
- Don't bundle runtime binaries (PHP, MySQL, …) into the app or the repo. (D-003)
- Don't add a dependency to do something a few lines of code can do.
- Don't start the next step or milestone without the owner saying "start work".
- Don't add Claude/AI attribution to commits or PRs.

## i18n quick reference

- English strings live in `src/i18n/locales/en.ts`. Use `const { t } = useTranslation()` and `t("feature.key")`. Keys are type-checked, so a typo fails `pnpm typecheck`.
- Add a key to `en.ts` first. Group by feature. Use `{{interpolation}}`, never concatenate translated fragments.
- A new language is a new file in `locales/` typed as `Translation` plus an entry in `src/i18n/index.ts`.
