# lunaRun

> A free, open-source, cross-platform local development environment for PHP & Laravel —
> run **multiple versions** of PHP, MySQL/MariaDB, Node.js and more **side by side**, and let lunaRun
> **read your project** and pick the right versions for you.

**Status:** 🚧 pre-alpha — under active development. Nothing is usable yet.

---

## Why lunaRun?

Classic stacks (XAMPP, WAMP, MAMP) give you **one** PHP and **one** MySQL. Real life is different:
one client is still on Laravel 9 / PHP 8.1, a new project is on Laravel 13 / PHP 8.4, and an old
WordPress site needs MySQL 5.7.

lunaRun lets you:

- **Install many versions** of each runtime (PHP 7.4 → 8.5, MySQL 8.0 / 8.4 / 9.x, MariaDB, Node LTS lines…) and keep them all.
- **Run them at the same time** — PHP 8.1 for one site and PHP 8.4 for another, simultaneously.
- **Point it at a project folder** — it reads `composer.json`, `composer.lock`, `package.json`,
  `.nvmrc`, `.env` and recommends (or downloads) the versions that project needs.
- **One click** to start the project's site, database and open a terminal with the right `php`, `composer` and `node` on the `PATH`.

## Platforms

| Platform | Status |
|---|---|
| Windows 10/11 (x64) | Primary target |
| macOS 12+ (Apple Silicon & Intel) | Primary target |
| Linux (x64, AppImage / .deb) | Best effort |

Installers are built automatically by GitHub Actions and published on the
[Releases](../../releases) page for every version tag.

## Documentation

| Doc | What's inside |
|---|---|
| [docs/PRD.md](docs/PRD.md) | Product requirements — what we build and why |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Tech stack, components, data model, IPC |
| [docs/RUNTIMES.md](docs/RUNTIMES.md) | Where binaries come from, registry format, install flow |
| [docs/PROJECT-DETECTION.md](docs/PROJECT-DETECTION.md) | How project requirements are detected and resolved |
| [docs/SERVICES.md](docs/SERVICES.md) | Process supervision, ports, config templates |
| [docs/BUILD-AND-RELEASE.md](docs/BUILD-AND-RELEASE.md) | CI, release pipeline, signing, auto-update |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phased task list |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Architecture decision log |
| [docs/REPO-SETUP.md](docs/REPO-SETUP.md) | Maintainer guide: GitHub setup & going public |

## Development

> Will be filled in during Phase 0 (see [ROADMAP](docs/ROADMAP.md)).

```bash
pnpm install
pnpm tauri dev
```

## License

[Apache License 2.0](LICENSE).

lunaRun does **not** bundle PHP, MySQL, MariaDB, Node.js or other runtimes. It downloads them on
your request from their official (or documented) sources, and each one keeps its own license.
See [docs/RUNTIMES.md](docs/RUNTIMES.md#licensing).

*lunaRun is not affiliated with Apache Friends (XAMPP), WampServer, Laravel, Oracle (MySQL) or the
MariaDB Foundation. Product names are used only to describe compatibility.*
