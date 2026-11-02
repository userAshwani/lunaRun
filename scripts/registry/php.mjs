// PHP: the official NTS x64 builds for Windows listed in windows.php.net's releases.json, plus the static
// builds for macOS and Linux (phpstatic.mjs), with the support dates from php.net (docs/RUNTIMES.md
// section 2). Both are versions of the same runtime: a version number that exists on both gets one entry
// with all its downloads, otherwise each platform family has its own versions (static builds lag).

import { checkedSha256, compareVersions, isoDate } from "./lib.mjs";
import { buildStaticPhp } from "./phpstatic.mjs";

export const PHP_RELEASES_URL = "https://windows.php.net/downloads/releases/";
const NTS_X64 = /^nts-((?:vc|vs)\d+)-x64$/;

/**
 * releases.json names a rebuild `8.3.36-1`. The `-1` is a Windows packaging revision, not a pre-release,
 * and the app would sort it *below* 8.3.36, so the registry says `8.3.36`. The real file name stays in the url.
 */
export function phpVersion(raw) {
  const match = /^(\d+\.\d+\.\d+)(?:-\d+)?$/.exec(raw);
  if (!match) throw new Error(`php: unexpected version ${JSON.stringify(raw)}`);
  return match[1];
}

/** php.net's support table to registry `lines`: end of security support is end of life. */
export function phpLines(states, wanted) {
  const lines = {};
  for (const byMajor of Object.values(states)) {
    for (const [line, info] of Object.entries(byMajor)) {
      if (!wanted.has(line)) continue;
      lines[line] = {
        eol: isoDate(info.security_support_end, `php ${line} security_support_end`),
        securityOnlySince: isoDate(info.active_support_end, `php ${line} active_support_end`),
      };
    }
  }
  return Object.fromEntries(Object.entries(lines).sort(([a], [b]) => compareVersions(b, a)));
}

/**
 * @param {{ phpReleases(): Promise<object>, phpStates(): Promise<object>, size(url: string): Promise<number | null> }} source
 * @param {{ previous?: object | null }} [options]
 */
export async function buildPhp(source, { previous = null } = {}) {
  const releases = await source.phpReleases();
  const states = await source.phpStates();

  const versions = [];
  for (const [line, release] of Object.entries(releases)) {
    if (!/^\d+\.\d+$/.test(line) || typeof release?.version !== "string") continue;
    const at = `php ${line}`;

    // Newest compiler first (vs17 before vs16), though a line normally has just one.
    const keys = Object.keys(release)
      .filter((key) => NTS_X64.test(key))
      .sort()
      .reverse();
    if (keys.length === 0) throw new Error(`${at}: no NTS x64 build in releases.json`);
    const build = release[keys[0]];
    const sha256 = checkedSha256(build.zip?.sha256, `${at} zip sha256`);
    const file = build.zip?.path;
    if (typeof file !== "string" || !file.endsWith(".zip")) throw new Error(`${at}: no zip path`);

    // Lines that are no longer current move to /archives/. Check where the file really is.
    let url = null;
    let size = null;
    for (const base of [PHP_RELEASES_URL, `${PHP_RELEASES_URL}archives/`]) {
      size = await source.size(base + file);
      if (size !== null) {
        url = base + file;
        break;
      }
    }
    if (url === null) throw new Error(`${at}: ${file} is not downloadable from windows.php.net`);

    versions.push({
      version: phpVersion(release.version),
      channel: "stable",
      releasedAt: isoDate(build.mtime, `${at} mtime`),
      artifacts: [
        {
          os: "windows",
          arch: "x86_64",
          variant: `nts-${NTS_X64.exec(keys[0])[1]}`,
          format: "zip",
          url,
          sha256,
          size,
          stripComponents: 0,
          requires: ["vcredist-14-x64"],
        },
      ],
    });
  }
  if (versions.length === 0) throw new Error("php: releases.json lists no versions");

  for (const extra of await buildStaticPhp(source, { previous })) {
    const same = versions.find((v) => v.version === extra.version);
    if (same) same.artifacts.push(...extra.artifacts);
    else versions.push(extra);
  }
  versions.sort((a, b) => compareVersions(b.version, a.version));

  const wanted = new Set(versions.map((v) => v.version.split(".").slice(0, 2).join(".")));
  return {
    displayName: "PHP",
    license: { name: "PHP License v3.01", url: "https://www.php.net/license/" },
    lines: phpLines(states, wanted),
    versions,
  };
}
