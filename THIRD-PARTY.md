# Third-party material in this repository

lunaRun itself is Apache-2.0 (see [LICENSE](LICENSE)). Dependencies installed by `pnpm` and `cargo` keep
their own licenses (see `package.json`, `Cargo.toml` and the lockfiles). This file lists third-party
**data and test material that is stored in the repository itself**.

| File | What it is | Source | License |
|---|---|---|---|
| `crates/luna-core/data/cacert.pem` | Mozilla's list of trusted certificate authorities, embedded in the app and given to every PHP lunaRun installs, because PHP on Windows has no certificate store of its own | Extracted from Mozilla's `certdata.txt` and published by curl at <https://curl.se/docs/caextract.html>. Refreshed with `pnpm ca-bundle`, which checks it against curl's published SHA-256 | MPL-2.0 (Mozilla Public License 2.0), stated in the file's own header. The file is included unmodified |
| `crates/luna-core/tests/fixtures/php/php.ini-development-8.3.36` | The development `php.ini` shipped in the PHP 8.3.36 Windows zip, used as a test fixture | <https://windows.php.net/downloads/releases/> | PHP License v3.01 |
| `scripts/registry/fixtures/` | Trimmed copies of public index files from php.net, nodejs.org, getcomposer.org, GitHub, mariadb.org and others, used as test fixtures | The upstream sites named in `docs/RUNTIMES.md` | Factual data, each as published by its upstream |

lunaRun does not bundle any runtime (PHP, Node.js, MySQL, ...): they are downloaded on the user's request and
verified before use (D-003).
