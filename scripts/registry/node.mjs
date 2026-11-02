// Node.js: the newest release of every major line from MIN_MAJOR up, with checksums from the
// official SHASUMS256.txt and end-of-life dates from the Node release schedule.

import { checkedSha256, compareVersions, isoDate, parseShasums } from "./lib.mjs";

/** Older majors than this are not offered. Projects pinned to Node 14 or older are rare and unsupported upstream for years. */
export const NODE_MIN_MAJOR = 16;
const DIST = "https://nodejs.org/dist";

/** In preference order for the app: how Node names the file, and where it goes in the registry. */
const TARGETS = [
  { suffix: "win-x64.zip", os: "windows", arch: "x86_64", format: "zip" },
  { suffix: "darwin-arm64.tar.gz", os: "macos", arch: "aarch64", format: "tar.gz" },
  { suffix: "darwin-x64.tar.gz", os: "macos", arch: "x86_64", format: "tar.gz" },
  { suffix: "linux-x64.tar.xz", os: "linux", arch: "x86_64", format: "tar.xz" },
  { suffix: "linux-arm64.tar.xz", os: "linux", arch: "aarch64", format: "tar.xz" },
];

/** The newest stable release of each major at or above `minMajor`: `[{ major, version, date }]`, newest major first. */
export function newestPerMajor(index, minMajor) {
  const best = new Map();
  for (const entry of index) {
    const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(entry.version); // no alpha, beta or rc
    if (!match) continue;
    const major = Number(match[1]);
    if (major < minMajor) continue;
    const version = entry.version.slice(1);
    const current = best.get(major);
    if (!current || compareVersions(version, current.version) > 0) {
      best.set(major, { major, version, date: entry.date });
    }
  }
  return [...best.values()].sort((a, b) => b.major - a.major);
}

/**
 * @param {{ nodeIndex(): Promise<object[]>, nodeSchedule(): Promise<object>, nodeShasums(version: string): Promise<string>, size(url: string): Promise<number | null> }} source
 */
export async function buildNode(source, { minMajor = NODE_MIN_MAJOR } = {}) {
  const newest = newestPerMajor(await source.nodeIndex(), minMajor);
  if (newest.length === 0) throw new Error("node: the index lists no stable release");
  const schedule = await source.nodeSchedule();

  const lines = {};
  const versions = [];
  for (const { major, version, date } of newest) {
    const at = `node ${version}`;

    const planned = schedule[`v${major}`];
    if (planned?.end) {
      lines[String(major)] = {
        eol: isoDate(planned.end, `node v${major} end`),
        ...(planned.maintenance && {
          securityOnlySince: isoDate(planned.maintenance, `node v${major} maintenance`),
        }),
      };
    }

    const sums = parseShasums(await source.nodeShasums(version));
    const artifacts = [];
    for (const target of TARGETS) {
      const file = `node-v${version}-${target.suffix}`;
      const sha = sums.get(file);
      if (!sha) continue; // e.g. no Apple silicon build before Node 16
      const url = `${DIST}/v${version}/${file}`;
      const size = await source.size(url);
      if (size === null)
        throw new Error(`${at}: ${file} is listed in SHASUMS256.txt but not downloadable`);
      artifacts.push({
        os: target.os,
        arch: target.arch,
        format: target.format,
        url,
        sha256: checkedSha256(sha, `${at} ${file}`),
        size,
        stripComponents: 1, // every Node archive wraps its files in node-v<version>-<platform>/
      });
    }
    if (artifacts.length === 0)
      throw new Error(`${at}: SHASUMS256.txt lists none of the expected files`);

    versions.push({
      version,
      channel: "stable",
      releasedAt: isoDate(date, `${at} date`),
      artifacts,
    });
  }

  return { displayName: "Node.js", lines, versions };
}
