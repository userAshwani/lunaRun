# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- App shell: sidebar navigation, hash routing, placeholder pages and a light/dark/system theme switcher.
- i18n with typed translation keys and an English strings file.
- ESLint (strict type-checked), Prettier, Vitest + Testing Library, clippy and rustfmt configs, and `pnpm lint`, `typecheck`, `test` scripts.
- Decisions D-011 (React Router, hash mode) and D-012 (ts-rs for generated IPC types); `.cargo/config.toml` export dir.
- Product name and bundle identifier constants in `luna-core`, with a test keeping `tauri.conf.json` in sync.
- Tauri 2 desktop shell with React, strict TypeScript, Vite, Tailwind CSS 4 and shadcn/ui; placeholder window.
- Cargo workspace with the `luna-core` crate and a pinned Rust toolchain.
- Product requirements, architecture, runtime registry, detection, services, build/release and roadmap documentation.
