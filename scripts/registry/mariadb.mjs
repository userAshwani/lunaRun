// MariaDB: the newest release of every long-term-support line, from the MariaDB Foundation's REST API
// (https://downloads.mariadb.org/rest-api/mariadb/). The API publishes a SHA-256 for every file, which is
// copied as it is. Downloads come from archive.mariadb.org, whose paths do not change, instead of the API's
// own links (those are plain http and redirect to a random mirror).
//
// There are no official macOS binaries and no Linux arm64 ones (docs/RUNTIMES.md section 2), so MariaDB is
// offered on Windows x64 and Linux x64 only. Rolling and preview lines are not offered.

import { checkedSha256, compareVersions, isoDate } from "./lib.mjs";

const ARCHIVE = "https://archive.mariadb.org";

/** Both archives hold one folder, `mariadb-<version>-<platform>/`. */
const TARGETS = [
  {
    file: (v) => `mariadb-${v}-winx64.zip`,
    dir: "winx64-packages",
    os: "windows",
    arch: "x86_64",
    format: "zip",
    requires: ["vcredist-14-x64"],
  },
  {
    file: (v) => `mariadb-${v}-linux-systemd-x86_64.tar.gz`,
    dir: "bintar-linux-systemd-x86_64",
    os: "linux",
    arch: "x86_64",
    format: "tar.gz",
    requires: [],
  },
];

/**
 * @param {{ mariadbMajors(): Promise<{ major_releases: object[] }>, mariadbLine(line: string): Promise<{ releases: object }>, size(url: string): Promise<number | null> }} source
 */
export async function buildMariadb(source) {
  const { major_releases: majors } = await source.mariadbMajors();
  const lts = majors.filter(
    (m) => m.release_status === "Stable" && m.release_support_type === "Long Term Support",
  );
  if (lts.length === 0) throw new Error("mariadb: the API lists no stable long-term-support line");

  const lines = {};
  const versions = [];
  for (const line of lts) {
    const at = `mariadb ${line.release_id}`;
    if (line.release_eol_date) {
      lines[line.release_id] = { eol: isoDate(line.release_eol_date, `${at} eol`) };
    }

    // Not every release has every build (11.8.7 has no Windows zip), so take the newest one that has any.
    const { releases } = await source.mariadbLine(line.release_id);
    const newestFirst = Object.entries(releases)
      .filter(([version]) => /^\d+\.\d+\.\d+$/.test(version))
      .sort(([a], [b]) => compareVersions(b, a));

    let found = false;
    for (const [version, release] of newestFirst) {
      const artifacts = [];
      for (const target of TARGETS) {
        const name = target.file(version);
        const file = release.files.find((f) => f.file_name === name);
        if (!file) continue;
        const url = `${ARCHIVE}/mariadb-${version}/${target.dir}/${name}`;
        const size = await source.size(url);
        if (size === null)
          throw new Error(`${at}: ${name} is listed by the API but not on archive.mariadb.org`);
        artifacts.push({
          os: target.os,
          arch: target.arch,
          format: target.format,
          url,
          sha256: checkedSha256(file.checksum?.sha256sum, `${at} ${name}`),
          size,
          stripComponents: 1,
          ...(target.requires.length > 0 && { requires: target.requires }),
        });
      }
      if (artifacts.length === 0) continue;
      versions.push({
        version,
        channel: "stable",
        releasedAt: isoDate(release.date_of_release, `${at} ${version} date`),
        artifacts,
      });
      found = true;
      break;
    }
    if (!found) throw new Error(`${at}: no release has a Windows or Linux archive`);
  }

  versions.sort((a, b) => compareVersions(b.version, a.version));
  return {
    displayName: "MariaDB",
    license: { name: "GPL v2", url: "https://mariadb.com/kb/en/mariadb-license/" },
    lines,
    versions,
  };
}
