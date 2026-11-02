// Shared by the generator tests: a source that answers from the real upstream samples in fixtures/.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { MYSQL_RELEASE_KEY } from "./mysql.mjs";
import { parseChecksums } from "./lib.mjs";

const dir = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");
export const text = (name) => readFileSync(path.join(dir, name), "utf8");
export const json = (name) => JSON.parse(text(name));

/**
 * `size` and `hash` are functions so a test can make files vanish or corrupt a download. By default
 * `hash` answers like an honest download of the real Caddy asset (the SHA-512 the authors published, the
 * SHA-256 GitHub shows, the real size).
 */
export function fakeSource({ size = (url) => url.length * 1000, hash, downloadVerified } = {}) {
  const assets = json("caddy-releases.json").flatMap((release) => release.assets);
  const honestHash = async (url) => {
    const file = url.split("/").pop();
    const version = /^caddy_(\d+\.\d+\.\d+)_/.exec(file)[1];
    const asset = assets.find((a) => a.name === file);
    const sha512 = parseChecksums(text(`caddy-checksums-${version}.txt`), 128).get(file);
    return {
      // Older releases have no GitHub digest: any fixed hex stands in for the hash of the download.
      sha256: asset.digest ? asset.digest.replace("sha256:", "") : sha512.slice(0, 64),
      sha512,
      size: asset.size,
    };
  };
  return {
    phpReleases: async () => json("php-releases.json"),
    phpStates: async () => json("php-states.json"),
    nodeIndex: async () => json("node-index.json"),
    nodeSchedule: async () => json("node-schedule.json"),
    nodeShasums: async (version) => text(`SHASUMS256-v${version}.txt`),
    mariadbMajors: async () => json("mariadb-majors.json"),
    mariadbLine: async (line) => json(`mariadb-${line}.json`),
    composerVersions: async () => json("composer-versions.json"),
    composerChecksum: async (path) =>
      text(`composer-sha256-${/\/download\/([^/]+)\//.exec(path)[1]}.txt`),
    caddyReleases: async () => json("caddy-releases.json"),
    caddyChecksums: async (tag, version) => text(`caddy-checksums-${version}.txt`),
    hash: hash ?? honestHash,
    mysqlCycles: async () => json("endoflife-mysql.json"),
    // An honest signed download: the pinned signer, the size the server announced, a hash that depends on the url.
    downloadVerified:
      downloadVerified ??
      (async (url) => ({
        sha256: createHash("sha256").update(url).digest("hex"),
        size: size(url),
        signer: MYSQL_RELEASE_KEY,
      })),
    size: async (url) => size(url),
  };
}
