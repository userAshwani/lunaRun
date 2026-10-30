# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- GitHub issue forms (bug report, feature request), an issue chooser that routes security reports to `SECURITY.md`, and a pull request template.
- `pnpm bump <version>` sets the version in `package.json`, `tauri.conf.json`, Cargo and `Cargo.lock` and promotes the changelog's Unreleased section, with dry-run, validation and rollback.
- Release workflow: a version tag builds Windows, macOS (universal) and Linux installers into a draft GitHub release; manual runs build one platform as downloadable artifacts. `scripts/check-versions.mjs` refuses a release whose tag and app versions disagree. Windows installer is per-user (no admin).
- Continuous integration (GitHub Actions): lint, tests, generated-types check and a debug app build on Ubuntu; manual Windows/macOS test matrix. Pinned pnpm via `packageManager`.
- First end-to-end command: `app_info` returns version, OS, architecture and data folder to the Diagnostics page, which has loading, error and retry states and a "Copy report" button. Commands are granted individually through generated permissions.
- `luna-core` foundation: typed `LunaError` with stable codes (TypeScript bindings generated), `LunaHome` with guarded `safe_remove`, and atomic, schema-versioned JSON state with corruption recovery and migrations.
- UI design system: light-first theme, Inter font, design tokens, grouped sidebar, dashboard overview, shared Card, PageHeader, EmptyState and StatCard components.
- Project tracker (`TRACKER.md`) for session handoff.
- App shell: sidebar navigation, hash routing, placeholder pages and a light/dark/system theme switcher.
- i18n with typed translation keys and an English strings file.
- ESLint (strict type-checked), Prettier, Vitest + Testing Library, clippy and rustfmt configs, and `pnpm lint`, `typecheck`, `test` scripts.
- Decisions D-011 (React Router, hash mode) and D-012 (ts-rs for generated IPC types); `.cargo/config.toml` export dir.
- Product name and bundle identifier constants in `luna-core`, with a test keeping `tauri.conf.json` in sync.
- Tauri 2 desktop shell with React, strict TypeScript, Vite, Tailwind CSS 4 and shadcn/ui; placeholder window.
- Cargo workspace with the `luna-core` crate and a pinned Rust toolchain.
- Product requirements, architecture, runtime registry, detection, services, build/release and roadmap documentation.
