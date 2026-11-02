// MySQL: the current release of every long-term-support line (and the end-of-life 8.0 line many existing
// projects still use). See D-017 for why it is built this way.
//
// MySQL publishes no machine-readable release list (dev.mysql.com answers 403 to scripts) and no SHA-256
// (only MD5 on those pages, plus detached GPG signatures). So:
//   - versions and end-of-life dates come from endoflife.date;
//   - download addresses follow MySQL's CDN naming, and HEAD says whether that build exists;
//   - a new download is streamed once through gpg and must be signed by MySQL's release key (the
//     fingerprint below is pinned), and only then is the SHA-256 computed from it pinned;
//   - a release already in the previous registry is not downloaded again.

import { checkedSha256, isoDate } from "./lib.mjs";

/** MySQL Release Engineering, 2023 key. The same fingerprint is on repo.mysql.com and on the keyservers. */
export const MYSQL_RELEASE_KEY = "BCA43417C3B485DD128EC6D4B7B3B788A8D3785C";
const CDN = "https://cdn.mysql.com/Downloads";

/**
 * Linux arm64 exists too (`linux-glibc2.28-aarch64.tar.xz`) but is left out: every build is downloaded
 * once to be verified and these are about 900 MB each. Add it here when it is wanted.
 */
const TARGETS = [
  {
    suffix: "winx64.zip",
    os: "windows",
    arch: "x86_64",
    format: "zip",
    requires: ["vcredist-14-x64"],
  },
  { suffix: "macos15-arm64.tar.gz", os: "macos", arch: "aarch64", format: "tar.gz", requires: [] },
  { suffix: "macos15-x86_64.tar.gz", os: "macos", arch: "x86_64", format: "tar.gz", requires: [] },
  {
    suffix: "linux-glibc2.28-x86_64.tar.xz",
    os: "linux",
    arch: "x86_64",
    format: "tar.xz",
    requires: [],
  },
];

/** The cycles worth offering: long-term support ones. endoflife.date gives `lts` as true or as the date LTS began. */
export function mysqlCycles(cycles) {
  return cycles.filter(
    (cycle) =>
      cycle.lts && /^\d+\.\d+$/.test(cycle.cycle) && cycle.latest?.startsWith(`${cycle.cycle}.`),
  );
}

/**
 * @param {{
 *   mysqlCycles(): Promise<object[]>,
 *   size(url: string): Promise<number | null>,
 *   downloadVerified(url: string, sigUrl: string): Promise<{ sha256: string, size: number, signer: string | null }>,
 * }} source
 * @param {{ previous?: object | null }} [options]
 */
export async function buildMysql(source, { previous = null } = {}) {
  const cycles = mysqlCycles(await source.mysqlCycles());
  if (cycles.length === 0)
    throw new Error("mysql: endoflife.date lists no long-term-support cycle");

  const lines = {};
  const versions = [];
  for (const cycle of cycles) {
    const version = cycle.latest;
    const at = `mysql ${version}`;
    if (typeof cycle.eol === "string")
      lines[cycle.cycle] = { eol: isoDate(cycle.eol, `${at} eol`) };
    const before = previous?.versions?.find((v) => v.version === version);

    const artifacts = [];
    for (const target of TARGETS) {
      const file = `mysql-${version}-${target.suffix}`;
      const url = `${CDN}/MySQL-${cycle.cycle}/${file}`;

      let sha256;
      let size;
      const known = before?.artifacts.find((a) => a.url === url);
      if (known) {
        ({ sha256, size } = known);
      } else {
        const announced = await source.size(url);
        if (announced === null) continue; // MySQL did not build this one (8.4.13 has no Intel macOS build)
        const got = await source.downloadVerified(url, `${url}.asc`);
        if (got.signer !== MYSQL_RELEASE_KEY) {
          throw new Error(
            `${at}: ${file} is not signed by the MySQL release key ${MYSQL_RELEASE_KEY}`,
          );
        }
        if (got.size !== announced)
          throw new Error(`${at}: ${file} is ${got.size} bytes, the server said ${announced}`);
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
        stripComponents: 1,
        ...(target.requires.length > 0 && { requires: target.requires }),
      });
    }
    if (artifacts.length === 0)
      throw new Error(`${at}: none of the expected files exists on cdn.mysql.com`);

    versions.push({
      version,
      channel: "stable",
      releasedAt: isoDate(cycle.latestReleaseDate, `${at} latestReleaseDate`),
      artifacts,
    });
  }

  return {
    displayName: "MySQL",
    license: { name: "GPL v2", url: "https://www.mysql.com/about/legal/licensing/oem/" },
    lines,
    versions,
  };
}
