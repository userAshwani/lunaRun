# Contributing to lunaRun

Thanks for your interest! lunaRun is in early development, so the details below may change.

## Ground rules

- Read [docs/PRD.md](docs/PRD.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first.
- Open an issue before starting a larger change, so we can agree on the approach.
- One logical change per PR. Reference requirement IDs (`FR-…`) and issues.
- Use [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`.
- CI must be green: `pnpm lint`, `pnpm test`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`.
- Add tests for new logic in `crates/luna-core`.

## Development setup

Prerequisites:

- Rust stable via [rustup](https://rustup.rs)
- Node.js 24 LTS and [pnpm](https://pnpm.io)
- Platform toolchain for Tauri:
  - **Windows:** Visual Studio Build Tools ("Desktop development with C++"), WebView2 (preinstalled on Windows 11)
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`)
  - **Linux:** `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf build-essential`

```bash
pnpm install
pnpm tauri dev
```

## License of contributions

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the same license as the project (inbound = outbound).
