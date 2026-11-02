// PHP for macOS and Linux: the static builds of static-php-cli (https://dl.static-php.dev, Q3 default
// "upstream first", docs/RUNTIMES.md section 2).
//
// What the real upstream looks like, and what that means here:
//   - Two downloads make one PHP: `php-<ver>-cli-<os>-<arch>.tar.gz` holds only `php`, and the matching
//     `php-<ver>-fpm-...` holds only `php-fpm`. The registry lists both, as the variants `static-cli` and
//     `static-fpm`, and the installer must fetch both. (`micro` builds exist and are ignored.)
//   - The `common` set is used because it has pdo_sqlite, pdo_mysql and pdo_pgsql (Laravel's databases).
//     It has no `intl`, `opcache` or `sodium`: the `bulk` set has those but no pdo_sqlite. Each artifact
//     carries the extension list, so the app can say so.
//   - static-php publishes **no checksum and no signature**. The SHA-256 here is computed from our own
//     download, so it protects against later tampering and transport damage but not against a bad build
//     at the source. That is the weakest link in the registry (docs/DECISIONS.md D-017).
//   - Builds lag the Windows releases (8.3.32 while windows.php.net has 8.3.36), so these versions are
//     separate registry entries unless a version number exists on both.
//   - Files can be rebuilt under the same name. `builtAt` (the listing's last-modified date) is stored on
//     every artifact; a file is downloaded again whenever that date differs from the previous registry.

import { checkedSha256, compareVersions, isoDate } from "./lib.mjs";

export const STATIC_PHP_BASE = "https://dl.static-php.dev/static-php-cli/common";
const FILE = /^php-(\d+\.\d+\.\d+)-(cli|fpm)-(linux|macos)-(x86_64|aarch64)\.tar\.gz$/;

/** Platform order is the preference order inside a version. */
const PLATFORMS = [
  { os: "macos", arch: "aarch64" },
  { os: "macos", arch: "x86_64" },
  { os: "linux", arch: "x86_64" },
  { os: "linux", arch: "aarch64" },
];
const SAPIS = ["cli", "fpm"];

/**
 * Group the listing into `Map<version, Map<"cli|macos|aarch64", { name, builtAt }>>`, ignoring directories,
 * `micro` builds and anything that is not a PHP archive.
 */
export function staticPhpFiles(listing) {
  const versions = new Map();
  for (const entry of listing) {
    const match = !entry.is_dir && FILE.exec(entry.name);
    if (!match) continue;
    const [, version, sapi, os, arch] = match;
    if (!versions.has(version)) versions.set(version, new Map());
    versions.get(version).set(`${sapi}|${os}|${arch}`, {
      name: entry.name,
      builtAt: isoDate(entry.last_modified, `${entry.name} last_modified`),
    });
  }
  return versions;
}

/** The platforms of a version that have both the CLI and the FPM archive. */
export function completePlatforms(files) {
  return PLATFORMS.filter((p) => SAPIS.every((sapi) => files.has(`${sapi}|${p.os}|${p.arch}`)));
}

/** The newest version of each minor line that has at least one complete platform, newest first. */
export function newestStaticPerLine(versions) {
  const best = new Map();
  for (const [version, files] of versions) {
    if (completePlatforms(files).length === 0) continue;
    const line = version.split(".").slice(0, 2).join(".");
    if (!best.has(line) || compareVersions(version, best.get(line)) > 0) best.set(line, version);
  }
  return [...best.values()].sort((a, b) => compareVersions(b, a));
}

/**
 * Registry versions (`{ version, channel, releasedAt, artifacts }`) for the static builds.
 *
 * @param {{ staticPhpListing(): Promise<object[]>, staticPhpExtensions(): Promise<string[]>, hash(url: string): Promise<{ sha256: string, size: number }> }} source
 * @param {{ previous?: object | null }} [options] the previous PHP runtime entry, for reuse of unchanged files
 */
export async function buildStaticPhp(source, { previous = null } = {}) {
  const grouped = staticPhpFiles(await source.staticPhpListing());
  const extensions = await source.staticPhpExtensions();
  if (!Array.isArray(extensions) || extensions.length === 0) {
    throw new Error("php (static): build-extensions.json lists no extension");
  }
  const chosen = newestStaticPerLine(grouped);
  if (chosen.length === 0) throw new Error("php (static): the listing has no complete PHP build");

  const versions = [];
  for (const version of chosen) {
    const at = `php ${version} (static)`;
    const files = grouped.get(version);
    const before = previous?.versions?.find((v) => v.version === version);

    const artifacts = [];
    for (const platform of completePlatforms(files)) {
      for (const sapi of SAPIS) {
        const { name, builtAt } = files.get(`${sapi}|${platform.os}|${platform.arch}`);
        const url = `${STATIC_PHP_BASE}/${name}`;
        const known = before?.artifacts.find((a) => a.url === url && a.builtAt === builtAt);

        let sha256;
        let size;
        if (known) {
          ({ sha256, size } = known);
        } else {
          const got = await source.hash(url);
          sha256 = got.sha256;
          size = got.size;
        }
        artifacts.push({
          os: platform.os,
          arch: platform.arch,
          variant: `static-${sapi}`,
          format: "tar.gz",
          url,
          sha256: checkedSha256(sha256, `${at} ${name}`),
          size,
          stripComponents: 0,
          extensions,
          builtAt,
        });
      }
    }
    const newest = artifacts
      .map((a) => a.builtAt)
      .sort()
      .at(-1);
    versions.push({ version, channel: "stable", releasedAt: newest, artifacts });
  }
  return versions;
}
