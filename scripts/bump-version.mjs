// Sets the app version everywhere it lives and prepares the changelog for a release.
//
//   pnpm bump 0.1.0                 # or 0.2.0-beta.1; no leading "v"
//   pnpm bump 0.1.0 --dry-run       # show what would change, write nothing
//   pnpm bump 0.0.9 --force         # allow a version that is not greater than the current one
//   pnpm bump 0.1.0 --no-lock       # do not refresh Cargo.lock (used by tests)
//   pnpm bump 0.1.0 --root <dir>    # operate on another checkout (used by tests)
//
// It updates package.json, src-tauri/tauri.conf.json, the Cargo [workspace.package] version and
// Cargo.lock, and turns "## [Unreleased]" in CHANGELOG.md into "## [x.y.z] - date" with a fresh empty
// Unreleased section above it. Every edit is computed in memory first, so a problem (bad version, empty
// changelog) changes nothing; if a later stage fails, the files are put back.
//
// Plain .mjs on purpose, like check-versions.mjs: it must run on a bare Node.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { findProblems, parseCargoWorkspaceVersion, readVersions } from "./check-versions.mjs";

const VERSION = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/;
const NUMERIC = /^\d+$/;

const eolOf = (text) => (text.includes("\r\n") ? "\r\n" : "\n");

/** @returns {{ core: number[], pre: string[] } | null} */
export function parseVersion(version) {
  const match = VERSION.exec(version);
  if (!match) return null;
  return {
    core: [Number(match[1]), Number(match[2]), Number(match[3])],
    pre: match[4] ? match[4].split(".") : [],
  };
}

/** SemVer precedence: negative if a < b, 0 if equal, positive if a > b. */
export function compareVersions(a, b) {
  const left = parseVersion(a);
  const right = parseVersion(b);
  if (!left || !right) throw new Error(`cannot compare "${a}" and "${b}"`);

  for (let i = 0; i < 3; i++) {
    if (left.core[i] !== right.core[i]) return left.core[i] - right.core[i];
  }
  // A release is greater than any prerelease of the same version.
  if (left.pre.length === 0 || right.pre.length === 0) {
    return Math.sign(right.pre.length - left.pre.length);
  }
  for (let i = 0; i < Math.max(left.pre.length, right.pre.length); i++) {
    const x = left.pre[i];
    const y = right.pre[i];
    if (x === undefined) return -1; // fewer identifiers is lower
    if (y === undefined) return 1;
    if (x === y) continue;
    const xNum = NUMERIC.test(x);
    const yNum = NUMERIC.test(y);
    if (xNum && yNum) return Number(x) - Number(y);
    if (xNum) return -1; // numeric identifiers are lower than alphanumeric ones
    if (yNum) return 1;
    return x < y ? -1 : 1;
  }
  return 0;
}

/** Replace the top-level "version" of a JSON file without reformatting anything else. */
export function setJsonVersion(text, version, label) {
  const pattern = /^(\s*"version"\s*:\s*")[^"]*(")/m;
  if (!pattern.test(text)) throw new Error(`${label}: no "version" field found`);
  const next = text.replace(pattern, (_, before, after) => `${before}${version}${after}`);
  if (JSON.parse(next).version !== version) {
    throw new Error(`${label}: could not update the top-level version safely`);
  }
  return next;
}

/** Replace `version` under [workspace.package] and nothing else in Cargo.toml. */
export function setCargoWorkspaceVersion(text, version) {
  const pattern = /(\[workspace\.package\][^[]*?^[ \t]*version[ \t]*=[ \t]*")[^"]*(")/m;
  if (!pattern.test(text)) throw new Error("Cargo.toml: no version under [workspace.package]");
  const next = text.replace(pattern, (_, before, after) => `${before}${version}${after}`);
  if (parseCargoWorkspaceVersion(next) !== version) {
    throw new Error("Cargo.toml: could not update the workspace version safely");
  }
  return next;
}

/** Turn "## [Unreleased]" into a dated release section and leave a fresh, empty Unreleased. */
export function promoteChangelog(text, version, date) {
  const newline = eolOf(text);
  const source = text.replace(/\r\n/g, "\n");

  const heading = /^## \[Unreleased\][ \t]*\n/m.exec(source);
  if (!heading) throw new Error('CHANGELOG.md: no "## [Unreleased]" heading found');

  const afterHeading = heading.index + heading[0].length;
  const rest = source.slice(afterHeading);
  const nextRelease = /^## \[/m.exec(rest);
  const body = (nextRelease ? rest.slice(0, nextRelease.index) : rest).trim();
  if (!body) {
    throw new Error("CHANGELOG.md: [Unreleased] is empty. Write the release notes first.");
  }

  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (new RegExp(`^## \\[${escaped}\\]`, "m").test(source)) {
    throw new Error(`CHANGELOG.md: a section for ${version} already exists`);
  }

  const older = nextRelease ? `\n${rest.slice(nextRelease.index)}` : "";
  const result =
    `${source.slice(0, heading.index)}## [Unreleased]\n\n` +
    `## [${version}] - ${date}\n\n${body}\n${older}`;
  return result.replace(/\n/g, newline);
}

const FILES = {
  packageJson: "package.json",
  tauriConf: "src-tauri/tauri.conf.json",
  cargoToml: "Cargo.toml",
  changelog: "CHANGELOG.md",
  cargoLock: "Cargo.lock",
};

/**
 * @param {object} options
 * @param {string} options.root repository root
 * @param {string} options.version new version, without a leading "v"
 * @param {string} options.date YYYY-MM-DD for the changelog heading
 * @param {boolean} [options.dryRun]
 * @param {boolean} [options.force] allow a version that is not greater than the current one
 * @param {boolean} [options.updateLock] refresh Cargo.lock with `cargo update --workspace`
 * @param {typeof spawnSync} [options.run] injected for tests
 */
export function bump({
  root,
  version,
  date,
  dryRun = false,
  force = false,
  updateLock = true,
  run = spawnSync,
}) {
  if (!parseVersion(version)) {
    throw new Error(
      `"${version}" is not a valid version. Use MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-prerelease, without a leading "v".`,
    );
  }

  const at = (name) => path.join(root, FILES[name]);
  const original = {};
  for (const name of ["packageJson", "tauriConf", "cargoToml", "changelog"]) {
    original[name] = readFileSync(at(name), "utf8");
  }
  original.cargoLock = existsSync(at("cargoLock")) ? readFileSync(at("cargoLock"), "utf8") : null;

  const current = JSON.parse(original.packageJson).version;
  if (!force && compareVersions(version, current) <= 0) {
    throw new Error(
      `${version} is not greater than the current version ${current}. Use --force to allow it.`,
    );
  }

  // Everything is computed before anything is written.
  const next = {
    packageJson: setJsonVersion(original.packageJson, version, FILES.packageJson),
    tauriConf: setJsonVersion(original.tauriConf, version, FILES.tauriConf),
    cargoToml: setCargoWorkspaceVersion(original.cargoToml, version),
    changelog: promoteChangelog(original.changelog, version, date),
  };
  const summary = { current, version, dryRun, files: Object.keys(next).map((name) => FILES[name]) };
  if (dryRun) return summary;

  const restore = () => {
    for (const name of Object.keys(next)) writeFileSync(at(name), original[name]);
    if (original.cargoLock !== null) writeFileSync(at("cargoLock"), original.cargoLock);
  };

  try {
    for (const [name, content] of Object.entries(next)) writeFileSync(at(name), content);

    if (updateLock) {
      const result = run("cargo", ["update", "--workspace", "--offline"], {
        cwd: root,
        encoding: "utf8",
      });
      if (result.error || result.status !== 0) {
        const reason = result.error ? result.error.message : result.stderr;
        throw new Error(`cargo update --workspace failed, nothing was changed.\n${reason}`);
      }
      summary.files.push(FILES.cargoLock);
    }

    const problems = findProblems(readVersions(root), `v${version}`);
    if (problems.length > 0) throw new Error(`consistency check failed:\n${problems.join("\n")}`);
  } catch (error) {
    restore();
    throw error;
  }
  return summary;
}

function parseArguments(argv) {
  const options = { dryRun: false, force: false, updateLock: true, root: undefined, version: "" };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--dry-run") options.dryRun = true;
    else if (arg === "--force") options.force = true;
    else if (arg === "--no-lock") options.updateLock = false;
    else if (arg === "--root") options.root = argv[++i];
    else if (arg.startsWith("--")) throw new Error(`unknown option ${arg}`);
    else if (!options.version) options.version = arg;
    else throw new Error(`unexpected argument ${arg}`);
  }
  if (!options.version) throw new Error("usage: pnpm bump <version> [--dry-run] [--force]");
  return options;
}

function main() {
  try {
    const options = parseArguments(process.argv.slice(2));
    const root = options.root ?? path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const date = new Date().toLocaleDateString("en-CA"); // YYYY-MM-DD in local time
    const summary = bump({ ...options, root, date });

    const verb = summary.dryRun ? "Would change" : "Changed";
    console.log(`${verb} version ${summary.current} -> ${summary.version} in:`);
    for (const file of summary.files) console.log(`  ${file}`);
    if (!summary.dryRun) {
      console.log("\nNext: review `git diff`, commit as");
      console.log(`  chore(release): v${summary.version}`);
      console.log(
        `then tag and push:  git tag v${summary.version} && git push origin v${summary.version}`,
      );
    }
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
