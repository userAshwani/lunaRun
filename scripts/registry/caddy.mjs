// Caddy: the newest stable release of each of the latest minor lines, from the GitHub releases of
// caddyserver/caddy.
//
// Caddy publishes **SHA-512** checksums (`caddy_<ver>_checksums.txt`), but the app pins SHA-256. So a new
// download is fetched once, its SHA-512 must equal the one the authors published, GitHub's own SHA-256
// digest for the asset must agree, and only then is the SHA-256 we computed from those verified bytes
// pinned. A release already in the previous registry is not downloaded again: release assets do not change.

import { compareVersions, checkedSha256, isoDate, parseChecksums } from "./lib.mjs";

export const CADDY_KEEP_MINORS = 3;
const DOWNLOAD = "https://github.com/caddyserver/caddy/releases/download";

/** Files inside every Caddy archive sit at the top level (`caddy`, `LICENSE`, `README.md`). */
const TARGETS = [
  { suffix: "windows_amd64.zip", os: "windows", arch: "x86_64", format: "zip" },
  { suffix: "mac_arm64.tar.gz", os: "macos", arch: "aarch64", format: "tar.gz" },
  { suffix: "mac_amd64.tar.gz", os: "macos", arch: "x86_64", format: "tar.gz" },
  { suffix: "linux_amd64.tar.gz", os: "linux", arch: "x86_64", format: "tar.gz" },
  { suffix: "linux_arm64.tar.gz", os: "linux", arch: "aarch64", format: "tar.gz" },
];

/** The newest stable release of each minor line, newest first, at most `keep` lines. */
export function newestPerMinor(releases, keep) {
  const best = new Map();
  for (const release of releases) {
    const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(release.tag_name); // no beta or rc
    if (!match || release.draft || release.prerelease) continue;
    const line = `${match[1]}.${match[2]}`;
    const version = release.tag_name.slice(1);
    const current = best.get(line);
    if (!current || compareVersions(version, current.version) > 0) {
      best.set(line, { version, release });
    }
  }
  return [...best.values()].sort((a, b) => compareVersions(b.version, a.version)).slice(0, keep);
}

/**
 * @param {{
 *   caddyReleases(): Promise<object[]>,
 *   caddyChecksums(tag: string, version: string): Promise<string>,
 *   hash(url: string): Promise<{ sha256: string, sha512: string, size: number }>,
 * }} source
 * @param {{ previous?: object | null, keepMinors?: number }} [options]
 */
export async function buildCaddy(source, { previous = null, keepMinors = CADDY_KEEP_MINORS } = {}) {
  const chosen = newestPerMinor(await source.caddyReleases(), keepMinors);
  if (chosen.length === 0) throw new Error("caddy: GitHub lists no stable release");

  const versions = [];
  for (const { version, release } of chosen) {
    const at = `caddy ${version}`;
    const published = new Map(release.assets.map((asset) => [asset.name, asset]));
    const authors = parseChecksums(await source.caddyChecksums(release.tag_name, version), 128);
    const before = previous?.versions?.find((v) => v.version === version);

    const artifacts = [];
    for (const target of TARGETS) {
      const file = `caddy_${version}_${target.suffix}`;
      const asset = published.get(file);
      const sha512 = authors.get(file);
      if (!asset && !sha512) continue; // not built for this platform
      if (!asset || !sha512)
        throw new Error(`${at}: ${file} is missing from the release or from its checksums`);

      const url = `${DOWNLOAD}/${release.tag_name}/${file}`;
      const known = before?.artifacts.find((a) => a.url === url);
      let sha256;
      let size;
      if (known) {
        ({ sha256, size } = known);
      } else {
        const got = await source.hash(url);
        if (got.sha512 !== sha512) {
          throw new Error(`${at}: ${file} does not match the SHA-512 the Caddy authors published`);
        }
        if (asset.digest && asset.digest !== `sha256:${got.sha256}`) {
          throw new Error(`${at}: ${file} does not match the SHA-256 digest GitHub shows for it`);
        }
        if (got.size !== asset.size)
          throw new Error(`${at}: ${file} is ${got.size} bytes, GitHub says ${asset.size}`);
        sha256 = got.sha256;
        size = got.size;
      }
      artifacts.push({
        os: target.os,
        arch: target.arch,
        format: target.format,
        url,
        sha256: checkedSha256(sha256, `${at} ${file}`),
        size,
        stripComponents: 0,
      });
    }
    if (artifacts.length === 0)
      throw new Error(`${at}: the release has none of the expected files`);

    versions.push({
      version,
      channel: "stable",
      releasedAt: isoDate(release.published_at, `${at} published_at`),
      artifacts,
    });
  }

  // Caddy has one major line and no published end-of-life dates, so no `lines`.
  return {
    displayName: "Caddy",
    license: {
      name: "Apache License 2.0",
      url: "https://github.com/caddyserver/caddy/blob/master/LICENSE",
    },
    lines: {},
    versions,
  };
}
