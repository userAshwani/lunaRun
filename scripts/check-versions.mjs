// Guards releases: the version must be identical in package.json, src-tauri/tauri.conf.json and the
// Cargo workspace, and, when a tag is given, must equal the tag without its leading "v".
//
//   node scripts/check-versions.mjs          # the three files must agree
//   node scripts/check-versions.mjs v0.1.0   # ...and match the tag
//
// Plain .mjs on purpose: it runs on a bare Node in CI before anything else is installed.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const TAG_PATTERN = /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/;

/** `version` under `[workspace.package]` in the root Cargo.toml, or null if absent. */
export function parseCargoWorkspaceVersion(cargoToml) {
  const section = /\[workspace\.package\]([^[]*)/.exec(cargoToml);
  const version = section && /^\s*version\s*=\s*"([^"]+)"/m.exec(section[1]);
  return version ? version[1] : null;
}

/**
 * @param {Record<string, string | null>} versions file name -> version found in it
 * @param {string | undefined} tag e.g. "v0.1.0", optional
 * @returns {string[]} human-readable problems, empty when everything agrees
 */
export function findProblems(versions, tag) {
  const problems = [];
  for (const [file, version] of Object.entries(versions)) {
    if (!version) problems.push(`${file}: no version found`);
  }

  const found = Object.entries(versions).filter(([, version]) => version);
  if (new Set(found.map(([, version]) => version)).size > 1) {
    const list = found.map(([file, version]) => `${file}=${version}`).join(", ");
    problems.push(`versions differ: ${list}`);
  }

  if (tag) {
    const match = TAG_PATTERN.exec(tag);
    if (!match) {
      problems.push(`tag "${tag}" is not of the form vMAJOR.MINOR.PATCH[-prerelease]`);
    } else {
      for (const [file, version] of found) {
        if (version !== match[1]) {
          problems.push(`${file} is ${version} but the tag is ${tag}`);
        }
      }
    }
  }
  return problems;
}

export function readVersions(root) {
  const read = (relative) => readFileSync(path.join(root, relative), "utf8");
  return {
    "package.json": JSON.parse(read("package.json")).version ?? null,
    "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version ?? null,
    "Cargo.toml": parseCargoWorkspaceVersion(read("Cargo.toml")),
  };
}

function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const tag = process.argv[2] || undefined;
  const versions = readVersions(root);
  const problems = findProblems(versions, tag);
  if (problems.length > 0) {
    for (const problem of problems) console.error(`::error::${problem}`);
    process.exit(1);
  }
  console.log(
    `Version ${Object.values(versions)[0]} is consistent${tag ? ` and matches ${tag}` : ""}.`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
