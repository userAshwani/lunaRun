# lunaRun — Runtimes, Registry & Install Flow

Related: [PRD §7.1](PRD.md#71-runtime-manager) · [ARCHITECTURE](ARCHITECTURE.md) · [DECISIONS D-003, D-005, D-009](DECISIONS.md)

## 1. Principles

1. **lunaRun does not bundle runtimes.** The installer contains only the app. Runtimes are downloaded on the user's request (D-003).
2. **Upstream first.** Prefer official upstream binaries. Use third-party or self-built binaries only where upstream publishes none, and document that here.
3. **Pinned and verified.** Every artifact in the registry has a SHA-256. The app refuses to install anything without one.
4. **The registry is separate from the app.** New PHP/MySQL releases show up without an app release (D-009).

## 2. Binary sources

> Verify every URL pattern at implementation time. Upstreams change their layouts.

| Runtime | Windows x64 | macOS (arm64 / x64) | Linux x64 | Checksum source |
|---|---|---|---|---|
| **PHP** | Official NTS zips from `windows.php.net/downloads/releases/` (current) and `/archives/` (older). Machine-readable list: `releases/releases.json`. | Static builds made with **static-php-cli** (`crazywhalecc/static-php-cli`). Start with the upstream prebuilt binaries; move to our own `lunarun-binaries` CI builds if extensions or versions are missing (Q3). | Same as macOS | windows.php.net publishes sha256; static-php: computed and pinned by our registry generator |
| **Composer** | `getcomposer.org/download/<ver>/composer.phar` (version list: `getcomposer.org/versions`) | same | same | upstream `.sha256sum` |
| **Node.js** | `nodejs.org/dist/v<ver>/node-v<ver>-win-x64.zip` (index: `nodejs.org/dist/index.json`) | `…-darwin-arm64.tar.gz` / `…-darwin-x64.tar.gz` | `…-linux-x64.tar.xz` | `SHASUMS256.txt` |
| **MySQL** | Official "ZIP Archive" (`mysql-<ver>-winx64.zip`) from `dev.mysql.com` / `cdn.mysql.com` | Official macOS tar.gz (arm64 & x86_64) | Official linux-glibc tar.xz | Upstream gives MD5 (on pages that answer 403 to scripts) + detached GPG signatures. The generator verifies each new download's signature against MySQL's pinned release key, then pins the SHA-256 it computed (D-017). Versions and end-of-life dates come from `endoflife.date`; CDN files: `cdn.mysql.com/Downloads/MySQL-<line>/mysql-<ver>-winx64.zip`, `-macos15-arm64.tar.gz`, `-macos15-x86_64.tar.gz` (not every release has it), `-linux-glibc2.28-x86_64.tar.xz` (Linux arm64 exists but is not offered yet). Sizes: 165 to 990 MB |
| **MariaDB** | `archive.mariadb.org/mariadb-<ver>/winx64-packages/…zip` | **Not available in v1** (no official binaries) | `archive.mariadb.org/…/bintar-linux-systemd-x86_64/…tar.gz` (x86_64 only, 330 to 490 MB) | The REST API `downloads.mariadb.org/rest-api/mariadb/<line>/` lists every file with its SHA-256; lines and end-of-life dates come from `…/rest-api/mariadb/`. The generator offers the newest release of each long-term-support line |
| **Caddy** | GitHub releases `caddyserver/caddy` (`caddy_<ver>_windows_amd64.zip`) | `…_mac_arm64.tar.gz` / `…_mac_amd64.tar.gz` | `…_linux_amd64.tar.gz` | `caddy_<ver>_checksums.txt` |

**Later phases:** PostgreSQL (EDB binary zips for Windows/macOS; Linux TBD), Valkey (Redis-compatible,
BSD-licensed. Prefer it over Redis because of Redis's license change), Mailpit (GitHub releases),
Nginx (official Windows zips; macOS/Linux need self-built binaries), Xdebug (Windows DLLs from xdebug.org).

### PHP specifics

- **Windows:** use **NTS x64** builds (FastCGI via `php-cgi.exe`). The compiler tag (`vs16`, `vs17`) is stored as the `variant`.
  All need the **VC++ 2015–2022 x64 redistributable** (FR-RT-12). Detect it via
  `HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64` → `Installed = 1`, and link to Microsoft's official installer if it is missing.
- **macOS/Linux static builds:** include CLI + FPM SAPIs and the Laravel extension set at minimum:
  `bcmath ctype curl dom fileinfo filter gd iconv intl mbstring openssl pdo pdo_mysql pdo_pgsql pdo_sqlite
  phar posix redis session simplexml sodium sqlite3 tokenizer xml xmlreader xmlwriter zip zlib`.
  Static builds **cannot load shared extensions** (`extension=foo.so`), so the extension set is fixed per build (Q5).
  **What upstream static-php really offers (checked 2026-11-02, Q3 default in use):** the `common` set at
  `dl.static-php.dev/static-php-cli/common/` has `bcmath bz2 calendar ctype curl dom exif fileinfo filter ftp gd gmp
  iconv mbstring mbregex mysqlnd openssl pcntl pdo pdo_mysql pdo_pgsql pdo_sqlite pgsql phar posix redis session
  simplexml soap sockets sqlite3 tokenizer xml xmlreader xmlwriter zip zlib`: everything Laravel itself needs, but
  **no `intl`, `opcache` or `sodium`**. The `bulk` set has those but no `pdo_sqlite` or `pdo_pgsql`, so it is worse
  for Laravel (SQLite is its default database). Self-built binaries (Q3) could have the union. `php` and `php-fpm` are
  **two separate archives** per platform (one file each, `stripComponents: 0`), listed in the registry as the
  variants `static-cli` and `static-fpm`: an installer needs both. The newest build of each line lags the Windows
  releases (8.3.32 against 8.3.36), so the registry has separate versions per platform family unless a number
  exists on both. static-php publishes **no checksum and no signature**: the SHA-256 is computed from the
  generator's own download (D-017), and each artifact has `builtAt`, the listing's date, to notice a rebuilt file.
- **macOS arm64:** binaries must be at least ad-hoc signed or the kernel kills them. Post-install runs
  `codesign --force --sign - <binary>` on every executable that `codesign -v` reports as unsigned.

### Version coverage (v1 target)

| Runtime | Lines offered |
|---|---|
| PHP | Windows 7.4, 8.0 – 8.5 · macOS/Linux 8.1 – 8.5 (whatever static builds provide) |
| Composer | latest 2.x patch versions (plus 2.2 LTS for PHP < 7.2.5, not needed in v1) |
| Node.js | All active + maintenance LTS lines, plus Current |
| MySQL | 8.0, 8.4 LTS, latest 9.x innovation |
| MariaDB | Current LTS lines (e.g. 10.11, 11.4, 11.8) |
| Caddy | latest 2.x |

EOL versions stay available (legacy projects need them) but carry `"eol": true` (FR-RT-07).

## 3. Registry format

Hosted as a static JSON file. The URL is configurable in settings and defaults to the published registry
(see §5). A copy is bundled into the app as an offline fallback.

```jsonc
{
  "schema": 1,
  "generatedAt": "2026-10-29T00:00:00Z",
  "runtimes": {
    "php": {
      "displayName": "PHP",
      "license": { "name": "PHP License v3.01", "url": "https://www.php.net/license/" },
      "lines": {
        "8.3": { "eol": "2027-12-31", "securityOnlySince": "2025-12-31" },
        "8.1": { "eol": "2025-12-31" }
      },
      "versions": [
        {
          "version": "8.3.14",
          "channel": "stable",                      // stable | rc | beta
          "releasedAt": "2026-…",
          "artifacts": [
            {
              "os": "windows", "arch": "x86_64",
              "variant": "nts-vs16",
              "format": "zip",                      // zip | tar.gz | tar.xz | file
              "url": "https://windows.php.net/downloads/releases/php-8.3.14-nts-Win32-vs16-x64.zip",
              "sha256": "…64 hex…",
              "size": 31234567,
              "stripComponents": 0,                 // leading dirs to strip on extract
              "requires": ["vcredist-14-x64"]
            },
            {
              "os": "macos", "arch": "aarch64",
              "variant": "static",
              "format": "tar.gz",
              "url": "https://…",
              "sha256": "…",
              "size": 12345678,
              "extensions": ["bcmath", "ctype", "…"] // fixed set for static builds
            }
          ]
        }
      ]
    },
    "node":  { "…": "…" },
    "mysql": { "…": "…" }
  }
}
```

Rules:
- `os` ∈ `windows | macos | linux`, `arch` ∈ `x86_64 | aarch64`. On macOS the app picks the native arch and falls back to `x86_64` (Rosetta) only if no native artifact exists, with a visible note.
- Unknown fields MUST be ignored (forward compatibility). So are artifacts and versions the app cannot use (unknown `os`, `arch`, `format` or `channel`): they are skipped, never an error. Everything the app *does* use is validated strictly (https URL, 64-hex `sha256`, `size` > 0, safe names, no duplicates). A higher `schema` than supported → use the cached/bundled registry and prompt the user to update the app.
- Phase 2: the registry is signed (minisign/Ed25519). The app embeds the public key and rejects a remote registry with a bad signature.

## 4. Install flow

```
request install(runtime, version)
 │
 ├─ resolve artifact for (os, arch)            → NO_ARTIFACT_FOR_PLATFORM
 ├─ check prerequisites (requires[])           → PREREQUISITE_MISSING (e.g. VC++ redist)
 ├─ check free disk (size × 3)                 → INSUFFICIENT_DISK
 ├─ download → LUNA_HOME/downloads/<sha256>.part
 │     resumable (HTTP Range), progress events, cancellable, retry ×3 with backoff
 ├─ verify sha256                              → CHECKSUM_MISMATCH (delete file)
 ├─ extract → runtimes/<rt>/.staging-<uuid>/   (zip-slip guard, strip components)
 ├─ adapter.post_install()                     (php.ini, codesign, chmod +x, …)
 ├─ adapter.verify()                           (run `php -v`, `node -v`, `mysqld --version`, …)
 ├─ write install.json receipt
 ├─ atomic rename .staging-<uuid> → runtimes/<rt>/<version>/
 └─ delete download, emit job:done
```

Any failure → remove staging dir, emit `job:done {ok:false, error}`. The same version cannot be installed
twice concurrently (job deduplication by `(runtime, version, variant)`).

**Extraction safety** (`luna-core::extract`, D-016). Every archive is treated as hostile; nothing is ever written outside
the staging directory. Entries with absolute paths, drive letters, UNC paths, `..` components or NUL bytes are rejected; the
staging directory must be empty; nothing is written *through* a symlink the archive created; symlinks (Unix only) must be
relative and stay inside; hard links must point at an earlier regular file; devices and pipes are rejected; files are
created with `create_new`, so nothing is overwritten. Limits on total size (8 GiB), per-file size (4 GiB) and entry count
(250,000) stop decompression bombs. Only `rwx` permission bits are kept (the `+x` bit matters for `php` and `mysqld`).
Errors: `ARCHIVE_UNSAFE`, `ARCHIVE_CORRUPT`, `ARCHIVE_TOO_LARGE`, `CANCELLED`.

**Uninstall:** refuse while a service from it is running (`RUNTIME_IN_USE`, with a "stop and uninstall" option).
Warn when projects are bound to it (`runtime_usage`). Remove `runtimes/<rt>/<ver>` and `config/<rt>/<ver>`.
DB `data/` dirs are removed only with an explicit `deleteData = true` (FR-RT-06).

## 5. Registry generation & hosting

- A script (`scripts/registry/generate.mjs`, plain JavaScript on a bare Node like the other scripts; run it
  with `pnpm registry`) pulls upstream indexes (`releases.json`, `index.json`, GitHub releases API, …) and
  writes `registry/registry.json`. SHA-256 values are **copied from the upstream's own published checksums**;
  only a runtime whose upstream publishes none, or publishes a different hash (Caddy ships SHA-512), has its
  SHA-256 computed from a download, and then only after that download matched the upstream's own checksum
  (and GitHub's digest when it shows one). Release assets do not change, so a version already in the
  previous registry is not downloaded again. The Composer file is one `composer.phar`, listed once per
  platform with `format: "file"`. GitHub API calls use `GITHUB_TOKEN` when set (it raises the rate limit)
  and fail with a clear message when the limit is used up.
  Exact sizes come from HEAD requests, because upstream sizes are rounded ("32.6MB"). Output is deterministic
  (the `generatedAt` stamp only moves when the data does), and a runtime whose upstream fails keeps its
  previous entry while the run exits non-zero. PHP for Windows reports its rebuilds as `8.3.36-1`; the
  registry says `8.3.36` (the file name keeps the real name), since the app would read `-1` as a pre-release.
- A scheduled GitHub Actions workflow (`registry.yml`, daily) runs the generator and opens a PR (or
  commits to a `registry` branch) when something changed. A human merges it.
- **Hosting:** the app must fetch the registry **without authentication**. While the main repo is
  private, `raw.githubusercontent.com` and private release assets are **not** reachable by the app.
  Options, in order of preference:
  1. A small separate **public** repo `lunarun-registry` (only the JSON, served via GitHub Pages or raw).
  2. Until that exists, during development: the bundled fallback registry plus `LUNA_REGISTRY_URL` pointing at a local file.

## 6. Licensing

| Runtime | License | Notes |
|---|---|---|
| PHP | PHP License v3.01 | Static builds also contain libraries (OpenSSL, libxml2, ICU, …) with their own licenses. If we self-build and publish binaries, we must ship their license texts and avoid GPL-only libs (use libedit, not readline). |
| Composer | MIT | |
| Node.js | MIT (+ bundled deps) | |
| MySQL Community | GPLv2 (with FOSS exception) | We only download from Oracle on the user's request; we do not redistribute. |
| MariaDB | GPLv2 | same |
| Caddy | Apache-2.0 | |
| Valkey (later) | BSD-3-Clause | Prefer over Redis ≥ 7.4 (non-OSI / AGPL licensing). |

The UI MUST show each runtime's license name and link on its detail view.
lunaRun's own license (Apache-2.0) covers only lunaRun's code.
