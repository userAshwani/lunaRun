// Builds registry/registry.json: the list of runtime versions lunaRun can download (docs/RUNTIMES.md
// sections 3 and 5). It reads the upstream indexes, never trusts a download it made itself for a checksum
// (every SHA-256 comes from the upstream's own published list), and writes the same bytes for the same data.
//
//   node scripts/registry/generate.mjs                      # all runtimes, writes registry/registry.json
//   node scripts/registry/generate.mjs --only node          # refresh one runtime, keep the others as they are
//   node scripts/registry/generate.mjs --out some/file.json
//
// A runtime whose upstream fails keeps its previous entry, the error is printed, and the exit code is 1,
// so a broken upstream is loud but never wipes data out of the registry.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { buildCaddy } from "./caddy.mjs";
import { buildComposer } from "./composer.mjs";
import { registryText, validateRegistry } from "./lib.mjs";
import { buildMariadb } from "./mariadb.mjs";
import { buildNode } from "./node.mjs";
import { buildPhp } from "./php.mjs";

export const BUILDERS = {
  caddy: buildCaddy,
  composer: buildComposer,
  mariadb: buildMariadb,
  node: buildNode,
  php: buildPhp,
};
const DEFAULT_OUT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../registry/registry.json",
);

/**
 * @param {object} source what the builders read, see the live one below and the fake one in the tests
 * @param {{ previous?: object | null, only?: string[], now?: () => string }} [options]
 * @returns {Promise<{ registry: object, errors: { runtime: string, message: string }[] }>}
 */
export async function generate(
  source,
  { previous = null, only = Object.keys(BUILDERS), now = () => new Date().toISOString() } = {},
) {
  const runtimes = {};
  const errors = [];
  for (const [id, build] of Object.entries(BUILDERS)) {
    const carried = previous?.runtimes?.[id];
    if (!only.includes(id)) {
      if (carried) runtimes[id] = carried;
      continue;
    }
    try {
      runtimes[id] = await build(source, { previous: carried ?? null });
    } catch (error) {
      errors.push({ runtime: id, message: error instanceof Error ? error.message : String(error) });
      if (carried) runtimes[id] = carried;
    }
  }

  const registry = { schema: 1, generatedAt: "", runtimes };
  // The timestamp only moves when the data does, so the daily run produces no diff on a quiet day.
  registry.generatedAt =
    previous &&
    registryText({ ...registry, generatedAt: previous.generatedAt }) === registryText(previous)
      ? previous.generatedAt
      : now();

  const problems = validateRegistry(registry);
  if (problems.length > 0)
    throw new Error(`the generated registry breaks its own rules:\n- ${problems.join("\n- ")}`);
  return { registry, errors };
}

/** The real upstreams. Every request identifies lunaRun and fails on a bad HTTP status. */
export function liveSource(fetchFn = fetch) {
  const headers = {
    "user-agent": "lunaRun-registry-generator (+https://github.com/userAshwani/lunaRun)",
  };
  const get = async (url) => {
    // The GitHub API allows 60 unauthenticated requests an hour. A token (set automatically in Actions)
    // raises that. It is only ever sent to api.github.com and never printed.
    const token = process.env.GITHUB_TOKEN;
    const auth =
      token && new URL(url).hostname === "api.github.com"
        ? { authorization: `Bearer ${token}` }
        : {};
    const response = await fetchFn(url, { headers: { ...headers, ...auth } });
    if (response.status === 403 || response.status === 429) {
      if (response.headers.get("x-ratelimit-remaining") === "0") {
        throw new Error(
          `${url}: the GitHub API rate limit is used up, set GITHUB_TOKEN or wait an hour`,
        );
      }
    }
    if (!response.ok) throw new Error(`${url} answered HTTP ${response.status}`);
    return response;
  };
  return {
    phpReleases: async () =>
      (await get("https://windows.php.net/downloads/releases/releases.json")).json(),
    phpStates: async () => (await get("https://www.php.net/releases/states.php")).json(),
    nodeIndex: async () => (await get("https://nodejs.org/dist/index.json")).json(),
    nodeSchedule: async () =>
      (await get("https://raw.githubusercontent.com/nodejs/Release/main/schedule.json")).json(),
    nodeShasums: async (version) =>
      (await get(`https://nodejs.org/dist/v${version}/SHASUMS256.txt`)).text(),
    mariadbMajors: async () =>
      (await get("https://downloads.mariadb.org/rest-api/mariadb/")).json(),
    mariadbLine: async (line) =>
      (await get(`https://downloads.mariadb.org/rest-api/mariadb/${line}/`)).json(),
    composerVersions: async () => (await get("https://getcomposer.org/versions")).json(),
    composerChecksum: async (path) =>
      (await get(`https://getcomposer.org${path}.sha256sum`)).text(),
    caddyReleases: async () =>
      (await get("https://api.github.com/repos/caddyserver/caddy/releases?per_page=50")).json(),
    caddyChecksums: async (tag, version) =>
      (
        await get(
          `https://github.com/caddyserver/caddy/releases/download/${tag}/caddy_${version}_checksums.txt`,
        )
      ).text(),
    /** Download a file once and return its SHA-256, SHA-512 and exact size, for upstreams that publish neither SHA-256 nor a size. */
    hash: async (url) => {
      const response = await get(url);
      const sha256 = createHash("sha256");
      const sha512 = createHash("sha512");
      let size = 0;
      for await (const chunk of response.body) {
        sha256.update(chunk);
        sha512.update(chunk);
        size += chunk.length;
      }
      return { sha256: sha256.digest("hex"), sha512: sha512.digest("hex"), size };
    },
    /** Exact size in bytes from a HEAD request, or null when the file does not exist. */
    size: async (url) => {
      // "identity": the size of the file as the app downloads it. A compressed answer carries a different
      // length, or none (getcomposer.org leaves it out when gzip is negotiated).
      const response = await fetchFn(url, {
        method: "HEAD",
        headers: { ...headers, "accept-encoding": "identity" },
      });
      if (response.status === 404) return null;
      if (!response.ok) throw new Error(`${url} answered HTTP ${response.status}`);
      const length = Number(response.headers.get("content-length"));
      if (!Number.isInteger(length) || length <= 0)
        throw new Error(`${url} did not say how big it is`);
      return length;
    },
  };
}

function parseArgs(argv) {
  const options = { out: DEFAULT_OUT, only: Object.keys(BUILDERS) };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--out") options.out = path.resolve(argv[++i] ?? "");
    else if (argv[i] === "--only") options.only = (argv[++i] ?? "").split(",").filter(Boolean);
    else throw new Error(`unknown argument ${JSON.stringify(argv[i])}`);
  }
  const unknown = options.only.filter((id) => !(id in BUILDERS));
  if (unknown.length > 0)
    throw new Error(
      `unknown runtime ${unknown.join(", ")} (known: ${Object.keys(BUILDERS).join(", ")})`,
    );
  return options;
}

async function main() {
  const { out, only } = parseArgs(process.argv.slice(2));
  const previous = existsSync(out) ? JSON.parse(readFileSync(out, "utf8")) : null;
  const { registry, errors } = await generate(liveSource(), { previous, only });

  const text = registryText(registry);
  if (!previous || registryText(previous) !== text) {
    mkdirSync(path.dirname(out), { recursive: true });
    writeFileSync(out, text);
    console.log(`wrote ${path.relative(process.cwd(), out)}`);
  } else {
    console.log("registry unchanged");
  }
  for (const [id, runtime] of Object.entries(registry.runtimes)) {
    console.log(
      `  ${id}: ${runtime.versions.length} versions, newest ${runtime.versions[0]?.version}`,
    );
  }
  for (const { runtime, message } of errors) console.error(`FAILED ${runtime}: ${message}`);
  if (errors.length > 0) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
