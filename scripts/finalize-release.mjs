// Turns the draft release created by the build into a public one with download links that never change.
//
//   node scripts/finalize-release.mjs v0.1.0 --publish     # rename installers, then publish
//   node scripts/finalize-release.mjs v0.1.0 --dry-run     # only show what would happen
//
// Tauri names files with the version (lunaRun_0.1.0_x64-setup.exe). GitHub's permanent
// ".../releases/latest/download/<name>" links need a fixed name, so every installer is renamed to a stable
// one, for example lunaRun-windows-x64-setup.exe. The README links to those names, so it never needs an
// edit when a new version is released. The version stays visible in the release title and tag.
//
// Nothing is renamed or published unless every expected installer is present, so a half-finished build
// can never become a public release. Needs the GitHub CLI (`gh`) with a token that may edit releases.
//
// Plain .mjs on purpose, like the other scripts: it runs on a bare Node.
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

/** Every installer a complete release must contain, and how to recognise it among Tauri's file names. */
export const INSTALLERS = [
  { stable: "lunaRun-windows-x64-setup.exe", match: /-setup\.exe$/ },
  { stable: "lunaRun-windows-x64.msi", match: /\.msi$/ },
  { stable: "lunaRun-macos-universal.dmg", match: /\.dmg$/ },
  { stable: "lunaRun-macos-universal.app.tar.gz", match: /\.app\.tar\.gz$/ },
  { stable: "lunaRun-linux-x64.AppImage", match: /\.AppImage$/ },
  { stable: "lunaRun-linux-x64.deb", match: /\.deb$/ },
  { stable: "lunaRun-linux-x64.rpm", match: /\.rpm$/ },
];

/**
 * @param {string[]} names current asset names of the release
 * @returns {{ renames: {from: string, to: string}[], problems: string[] }} what to rename, and why not to proceed
 */
export function planRenames(names) {
  const renames = [];
  const problems = [];
  const claimed = new Set();

  for (const { stable, match } of INSTALLERS) {
    if (names.includes(stable)) {
      claimed.add(stable); // already renamed by an earlier run
      continue;
    }
    const found = names.filter((name) => match.test(name) && !claimed.has(name));
    if (found.length === 0) {
      problems.push(`missing: no asset matches ${stable}`);
    } else if (found.length > 1) {
      problems.push(`ambiguous: ${found.join(", ")} all match ${stable}`);
    } else {
      claimed.add(found[0]);
      renames.push({ from: found[0], to: stable });
    }
  }

  const stableNames = new Set(INSTALLERS.map((installer) => installer.stable));
  for (const name of names) {
    if (!claimed.has(name) && !stableNames.has(name)) {
      problems.push(`unexpected asset ${name}`);
    }
  }
  return { renames, problems };
}

function gh(run, args) {
  const result = run("gh", args, { encoding: "utf8" });
  if (result.error) throw new Error(`cannot run gh: ${result.error.message}`);
  if (result.status !== 0) {
    throw new Error(`gh ${args.slice(0, 3).join(" ")} failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout;
}

/**
 * @param {object} options
 * @param {string} options.tag e.g. "v0.1.0"
 * @param {string} options.repo "owner/name"
 * @param {boolean} [options.publish] publish the release after renaming
 * @param {boolean} [options.dryRun] change nothing
 * @param {typeof spawnSync} [options.run] injected for tests
 */
export function finalizeRelease({ tag, repo, publish = false, dryRun = false, run = spawnSync }) {
  const release = JSON.parse(
    gh(run, ["release", "view", tag, "--repo", repo, "--json", "assets,isDraft,isPrerelease"]),
  );
  const assets = release.assets;
  const { renames, problems } = planRenames(assets.map((asset) => asset.name));
  if (problems.length > 0) {
    throw new Error(
      `release ${tag} is not complete, nothing was changed:\n  ${problems.join("\n  ")}`,
    );
  }

  const summary = {
    tag,
    renames,
    willPublish: publish && release.isDraft,
    prerelease: release.isPrerelease,
  };
  if (dryRun) return { ...summary, dryRun: true };

  for (const { from, to } of renames) {
    const asset = assets.find((candidate) => candidate.name === from);
    const id = /\/assets\/(\d+)$/.exec(asset.apiUrl ?? "")?.[1];
    if (!id) throw new Error(`cannot find the asset id of ${from}`);
    gh(run, [
      "api",
      "--method",
      "PATCH",
      `repos/${repo}/releases/assets/${id}`,
      "-f",
      `name=${to}`,
    ]);
  }
  if (summary.willPublish) {
    gh(run, ["release", "edit", tag, "--repo", repo, "--draft=false"]);
  }
  return { ...summary, dryRun: false };
}

function main() {
  try {
    const args = process.argv.slice(2);
    const tag = args.find((arg) => !arg.startsWith("--"));
    if (!tag)
      throw new Error(
        "usage: node scripts/finalize-release.mjs <tag> [--publish] [--dry-run] [--repo owner/name]",
      );
    const repoFlag = args.indexOf("--repo");
    const repo = repoFlag >= 0 ? args[repoFlag + 1] : process.env.GITHUB_REPOSITORY;
    if (!repo) throw new Error("no repository: pass --repo owner/name or set GITHUB_REPOSITORY");

    const result = finalizeRelease({
      tag,
      repo,
      publish: args.includes("--publish"),
      dryRun: args.includes("--dry-run"),
    });
    console.log(
      `${result.dryRun ? "Would rename" : "Renamed"} ${result.renames.length} asset(s) of ${result.tag}:`,
    );
    for (const { from, to } of result.renames) console.log(`  ${from} -> ${to}`);
    if (result.willPublish) {
      console.log(
        result.dryRun
          ? "Would publish the release."
          : `Published ${result.tag}${result.prerelease ? " (pre-release)" : ""}.`,
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
