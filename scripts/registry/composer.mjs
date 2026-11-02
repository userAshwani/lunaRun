// Composer: the releases listed on getcomposer.org/versions. Composer is one `composer.phar` for every
// platform, so the same file is listed once per supported OS and architecture (the registry has no
// "any platform" entry, and the app picks by OS and architecture). `format: "file"` means "do not extract".
//
// The checksum is the SHA-256 that getcomposer.org publishes next to each file.

import { checkedSha256, compareVersions, isoDate, parseChecksums } from "./lib.mjs";

const SITE = "https://getcomposer.org";
const PLATFORMS = [
  { os: "windows", arch: "x86_64" },
  { os: "macos", arch: "aarch64" },
  { os: "macos", arch: "x86_64" },
  { os: "linux", arch: "x86_64" },
  { os: "linux", arch: "aarch64" },
];

/**
 * Every real release the page names: the `stable` list and the per-line lists (`1`, `2`, `2.2`).
 * `preview` and `snapshot` are not releases. Returns `[{ version, path }]`, newest first.
 */
export function composerReleases(data) {
  const found = new Map();
  for (const [key, entries] of Object.entries(data)) {
    if (key !== "stable" && !/^\d+(\.\d+)?$/.test(key)) continue;
    for (const entry of entries) {
      const match = /^\/download\/(\d+\.\d+\.\d+)\/composer\.phar$/.exec(entry.path ?? "");
      if (match && match[1] === entry.version) found.set(match[1], entry.path);
    }
  }
  return [...found]
    .map(([version, path]) => ({ version, path }))
    .sort((a, b) => compareVersions(b.version, a.version));
}

/** A line is `1` or `2.2`; it has an end date only when Composer publishes one (`maintenance-until`). */
export function composerLines(data) {
  const lines = {};
  for (const [key, entries] of Object.entries(data)) {
    if (!/^\d+(\.\d+)?$/.test(key)) continue;
    const entry = entries.find((e) => e.version?.startsWith(`${key}.`));
    if (entry?.["maintenance-until"]) {
      lines[key] = {
        eol: isoDate(entry["maintenance-until"], `composer ${key} maintenance-until`),
      };
    }
  }
  return lines;
}

/**
 * @param {{ composerVersions(): Promise<object>, composerChecksum(path: string): Promise<string>, size(url: string): Promise<number | null> }} source
 */
export async function buildComposer(source) {
  const data = await source.composerVersions();
  const releases = composerReleases(data);
  if (releases.length === 0) throw new Error("composer: getcomposer.org/versions lists no release");

  const versions = [];
  for (const { version, path } of releases) {
    const at = `composer ${version}`;
    const url = `${SITE}${path}`;
    const sums = parseChecksums(await source.composerChecksum(path), 64);
    const sha256 = checkedSha256(sums.get("composer.phar"), `${at} .sha256sum`);
    const size = await source.size(url);
    if (size === null) throw new Error(`${at}: ${url} is not downloadable`);
    versions.push({
      version,
      channel: "stable",
      artifacts: PLATFORMS.map((platform) => ({
        ...platform,
        format: "file",
        url,
        sha256,
        size,
        stripComponents: 0,
      })),
    });
  }

  return {
    displayName: "Composer",
    license: { name: "MIT", url: "https://github.com/composer/composer/blob/main/LICENSE" },
    lines: composerLines(data),
    versions,
  };
}
