# lunaRun — Project Detection & Version Resolution

Related: [PRD §7.3](PRD.md#73-projects-and-detection-core-feature) · [ARCHITECTURE](ARCHITECTURE.md)

This is lunaRun's signature feature. It lives in `crates/luna-core/src/projects/` and
`crates/luna-core/src/constraints/`, with **no UI or OS dependencies**, so it can be fully unit-tested.

## 1. Contract

```rust
pub fn detect(project_dir: &Path, data: &DetectionData) -> DetectionReport;           // pure, read-only
pub fn resolve(report: &DetectionReport, installed: &[InstalledRuntime],
               available: &Registry, prefs: &Settings) -> Resolution;                  // pure
```

- `detect` only **reads** files (FR-PJ-05). It never runs project code, `composer` or `npm`.
- Missing or broken files never fail detection. They add a `Warning` and detection carries on.
- `DetectionData` holds the JSON data tables (Laravel→PHP map, extension names) so they can be updated without code changes (FR-PJ-07).

```rust
pub struct DetectionReport {
    pub framework: Option<Framework>,              // kind + version + evidence
    pub doc_root: DocRoot,                         // relative path + evidence
    pub php: Option<Requirement>,                  // constraint + sources
    pub php_extensions: Vec<ExtRequirement>,       // name + which package needs it
    pub node: Option<Requirement>,
    pub database: Option<DbHint>,                  // engine, port, db name (no password)
    pub services_hints: Vec<ServiceHint>,          // redis, mailpit, … (Phase 2)
    pub warnings: Vec<Warning>,
    pub inputs_hash: String,                       // hash of all input files, for change detection
}
pub struct Requirement {
    pub constraint: Constraint,                    // combined (intersection) of all sources
    pub preferred: Option<VersionReq>,             // e.g. from config.platform.php or .php-version
    pub sources: Vec<Source>,                      // file, json path, raw string, parsed constraint
}
```

## 2. PHP — inputs, in priority order

| # | Source | How to read | Meaning |
|---|---|---|---|
| 1 | **User override** (stored in the project record) | — | Wins over everything. Shown as "manual". |
| 2 | `.lunarun.json` → `php` (Phase 2) | JSON | Team-declared pin. Exact or line. |
| 3 | `.php-version` | first non-empty line, trimmed (e.g. `8.3`, `8.3.14`) | Pin (same convention as phpenv / other tools). |
| 4 | `composer.json` → `config.platform.php` | JSON | The PHP version the team **targets**. Use as `preferred`. |
| 5 | `composer.json` → `require.php` | Composer constraint | Root constraint. |
| 6 | `composer.lock` → every `packages[].require.php` and `packages-dev[].require.php` | Composer constraints | Intersected with #5. This is what `composer check-platform-reqs` checks, and it catches "your lock needs ≥ 8.2". |
| 7 | `composer.lock` → `platform.php` | Composer constraint | Equals the root require at lock time. Use if composer.json is missing. |
| 8 | Framework table (`data/laravel-php.json`) using the locked `laravel/framework` version | Data lookup | **Fallback only** when 4–7 give nothing. |
| 9 | Default | settings default PHP line, else latest installed | Shown as "no requirement found". |

**Combining:** the effective constraint is the **intersection** of every constraint from #5–#7 that is present.
Pins (#1–#3) replace the computed constraint, but if a pin falls outside it, add the warning
`PIN_CONFLICTS_WITH_COMPOSER`. If the intersection is empty, add `UNSATISFIABLE_PHP_CONSTRAINT`, list the
conflicting packages, and fall back to #5 alone.

### Laravel → PHP table (data file, verify each entry against laravel.com/docs before release)

```jsonc
// crates/luna-core/data/laravel-php.json
{ "schema": 1, "laravel": [
  { "framework": "^8.0",  "php": ">=7.3 <8.2" },
  { "framework": "^9.0",  "php": ">=8.0 <8.3" },
  { "framework": "^10.0", "php": ">=8.1 <8.4" },
  { "framework": "^11.0", "php": ">=8.2 <8.5" },
  { "framework": "^12.0", "php": ">=8.2 <8.6" },
  { "framework": "^13.0", "php": ">=8.3 <8.6" }
]}
```

### PHP extensions

- Collect every `ext-*` key from `composer.json` `require` / `require-dev` and from every package in `composer.lock`.
- Map Composer names to ini/loaded names through `data/php-extensions.json` (e.g. `ext-pdo_mysql` → `pdo_mysql`; `ext-zend-opcache` → `opcache` as a `zend_extension`). Note that some are built in and always present (`ext-json` on PHP 8+, `ext-ctype` …).
- Report each as `available-enabled | available-disabled | not-available-in-this-build` for the recommended version.
- Laravel baseline (always checked for Laravel projects): `ctype curl dom fileinfo filter hash mbstring openssl pcre pdo session tokenizer xml`.

## 3. Node — inputs, in priority order

| # | Source | Notes |
|---|---|---|
| 1 | User override | |
| 2 | `.lunarun.json` → `node` (Phase 2) | |
| 3 | `.nvmrc` | Accept `20`, `v20.11.0`, `20.11`, `lts/*`, `lts/<codename>` (e.g. `lts/jod`), `node` (= latest). Codename → line via registry metadata. |
| 4 | `.node-version` | same parser |
| 5 | `package.json` → `volta.node` | exact |
| 6 | `package.json` → `engines.node` | **npm semver** range syntax (not Composer syntax) |
| 7 | Default | latest installed LTS, else latest LTS from the registry |

No `package.json` → Node is "not required" (still offered, not recommended).
`packageManager` (e.g. `pnpm@9.x`) is reported as info: "enable via Corepack" (Phase 2).

## 4. Database hints

Read `.env`, falling back to `.env.example`. Parse with a minimal dotenv parser (quotes, comments,
`export` prefix). **Read only these keys:**
`DB_CONNECTION, DB_HOST, DB_PORT, DB_DATABASE, DB_USERNAME` (never `DB_PASSWORD`; never log values),
plus Phase 2 hints: `REDIS_HOST, CACHE_STORE, SESSION_DRIVER, QUEUE_CONNECTION, MAIL_HOST, MAIL_PORT`.

| `DB_CONNECTION` | Suggest |
|---|---|
| `mysql` | MySQL (default line: 8.4 LTS) |
| `mariadb` | MariaDB (Windows/Linux); on macOS suggest MySQL with a note |
| `pgsql` | PostgreSQL (Phase 2, otherwise "not supported yet") |
| `sqlite` | Nothing to run; check `pdo_sqlite` |
| `sqlsrv` | Not supported, info only |

If `DB_PORT` differs from the port of the bound DB service, show the fix "Update DB_PORT in .env". It needs
explicit confirmation with a diff preview, edits only that line, and keeps a `.env.lunarun-backup` (FR-PJ-05).
If `DB_DATABASE` does not exist on the bound server, offer "Create database".

## 5. Framework & document root

| Framework | Evidence | Doc root |
|---|---|---|
| Laravel | `artisan` file + `laravel/framework` in composer.lock (version from lock) | `public/` |
| Symfony (Phase 2) | `symfony/framework-bundle` in lock, `bin/console` | `public/` |
| WordPress (Phase 2) | `wp-config.php` or `wp-includes/version.php` (version from `$wp_version`) | project root |
| Generic PHP | `public/index.php` → `public/`; else `index.php` → root | as found |

The user can change the doc root (stored in the project record).

## 6. Constraint engine

Two parsers, one matching model (`VersionReq` = a union of intervals over `semver::Version`, with partial versions normalised):

### Composer syntax (PHP, Composer packages)

| Syntax | Meaning |
|---|---|
| `8.2.1`, `=8.2.1`, `v8.2.1` | exact |
| `>=8.1`, `>8.1`, `<8.4`, `<=8.3`, `!=8.2.0` | comparison |
| `>=8.1 <8.4` or `>=8.1, <8.4` | AND (space or comma) |
| `^7.4 \|\| ^8.0`, `^7.4 \| ^8.0` | OR (`\|\|` or single `\|`) |
| `8.1 - 8.3` | `>=8.1.0 <8.4.0` (partial upper bound acts as a wildcard) |
| `~8.1` | `>=8.1.0 <9.0.0` |
| `~8.1.2` | `>=8.1.2 <8.2.0` |
| `^8.1` | `>=8.1.0 <9.0.0` |
| `^0.3` | `>=0.3.0 <0.4.0` |
| `8.2.*`, `8.*`, `*` | wildcard |
| `@dev`, `@stable` suffixes, `dev-main` | stability flags: ignore flag; `dev-*` → unconstrained + warning |

### npm semver syntax (Node `engines`)
Standard node-semver: `^`, `~`, `x`/`*`, hyphen ranges, `||`, space-AND, `>=`. Use a well-tested crate
if one matches node-semver semantics; otherwise implement it and test against node-semver's published test cases.

## 7. Resolution algorithm

For each runtime with a requirement:

1. `candidates_installed` = installed versions that satisfy the constraint (stable channel only, unless the constraint explicitly allows pre-releases).
2. `candidates_available` = registry versions for this OS/arch that satisfy it.
3. If `preferred` is set (pin or `config.platform.php`): restrict both lists to that line (or exact version) when possible.
4. **Recommended** = the highest version in `candidates_installed`; if empty, the highest in `candidates_available`, flagged `needs_install`.
   - Tie-break: prefer non-EOL lines, then the user's default line from settings.
5. Binding suggestion: `mode = line` (e.g. `8.3`) unless the source was an exact pin.
6. Nothing satisfies it on this platform → `NO_COMPATIBLE_VERSION`, with an explanation (e.g. "PHP 7.4 is not available on macOS in lunaRun v1").

The UI shows the result as a checklist with one-click fixes (`project_apply_fix`):
`✓ PHP 8.3.14 installed — matches ^8.2 (composer.json) ∩ >=8.2 (laravel/framework 12.3)`,
`⚠ Missing extension: intl (needed by filament/filament) — [Enable]`,
`✗ Node 22 required (.nvmrc) — [Install Node 22.x & use]`.

## 8. Test fixtures

`crates/luna-core/tests/fixtures/projects/` holds minimal fake projects (only the relevant files, no vendor/ dirs):

| Fixture | Expectation |
|---|---|
| `laravel-10` | composer `^8.1` ∩ lock → recommend highest installed version that satisfies both |
| `laravel-10-no-php-require` | no `require.php`, locked `laravel/framework` 10.x → table fallback `>=8.1 <8.4` |
| `laravel-11-lock-needs-8.3` | root `^8.2` but a locked package needs `>=8.3` → constraint `>=8.3` |
| `laravel-12-platform-8.2` | `config.platform.php = 8.2.0` → preferred line 8.2 |
| `laravel-13` | PHP `>=8.3` |
| `legacy-php74` | `^7.4`, on macOS → `NO_COMPATIBLE_VERSION` |
| `pinned-php-version-file` | `.php-version` = `8.1`, composer says `^8.2` → pin wins + `PIN_CONFLICTS_WITH_COMPOSER` |
| `unsatisfiable` | root `^8.1 <8.2`, lock needs `>=8.2` → `UNSATISFIABLE_PHP_CONSTRAINT` |
| `or-constraint` | `^7.4 \|\| ^8.0` |
| `nvmrc-lts` | `.nvmrc` = `lts/*` |
| `engines-node` | `engines.node = ">=20 <23"` |
| `env-mariadb` | `DB_CONNECTION=mariadb`, `DB_PORT=3307` |
| `env-missing-uses-example` | only `.env.example` |
| `broken-composer-json` | invalid JSON → warning, no panic |
| `generic-php-root-index` | `index.php` at root → doc root `.` |
| `path with spaces ü` | the fixture folder name itself contains a space and a non-ASCII character |

Each fixture has an `expected.json` that the test compares against (golden tests).
