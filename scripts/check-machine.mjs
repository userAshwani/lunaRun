// Looks at the computer it runs on and says what is missing before lunaRun can be built, tested and pushed from it:
// the tools (git, Node, pnpm, Rust), what Tauri needs on this operating system, the repository itself (a clone, up to
// date, a git identity, the right to push) and the installed dependencies. It changes nothing: it prints one line per
// check with the command that fixes it, then READY or NOT READY (exit code 0 or 1). A new Claude session on a fresh
// PC runs it first and acts on the output (docs: TRACKER.md section 9).
//
//   node scripts/check-machine.mjs        # or: pnpm check:machine
//
// Plain .mjs with no dependencies on purpose: it must run on a bare Node, before `pnpm install` has happened.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
/** What the repo is, for the "is this the right clone" check. */
const REMOTE_PATTERN = /github\.com[:/]userAshwani\/lunaRun(\.git)?$/i;
/** The oldest Node this repo's tooling runs on (CI uses 24, the owner's machine 26). */
export const MIN_NODE_MAJOR = 22;

// ---------------------------------------------------------------------------------------------
// Pure helpers (tested in check-machine.test.mjs)
// ---------------------------------------------------------------------------------------------

/** `1.99.1` against `1.99`: negative, 0 or positive. Missing parts count as 0; anything after a `-` is ignored. */
export function compareVersions(a, b) {
  const parts = (v) =>
    String(v)
      .split("-")[0]
      .split(".")
      .map((n) => Number.parseInt(n, 10) || 0);
  const [left, right] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(left.length, right.length); i += 1) {
    const diff = (left[i] ?? 0) - (right[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

/** The first `1.2.3` found in a program's `--version` output, or null. */
export function versionIn(output) {
  const found = /(\d+\.\d+(?:\.\d+)?)/.exec(output ?? "");
  return found ? found[1] : null;
}

/** `rust-version` under `[workspace.package]` in the root Cargo.toml, or null. */
export function parseRustVersion(cargoToml) {
  const section = /\[workspace\.package\]([^[]*)/.exec(cargoToml);
  const found = section && /^\s*rust-version\s*=\s*"([^"]+)"/m.exec(section[1]);
  return found ? found[1] : null;
}

/** `12.16.1` from `"packageManager": "pnpm@12.16.1"` in package.json text, or null. */
export function parsePnpmVersion(packageJson) {
  try {
    const manager = JSON.parse(packageJson).packageManager ?? "";
    const found = /^pnpm@(\d+\.\d+\.\d+)/.exec(manager);
    return found ? found[1] : null;
  } catch {
    return null;
  }
}

/** `ahead`/`behind` from the output of `git rev-list --left-right --count HEAD...origin/main`. */
export function parseAheadBehind(output) {
  const found = /^\s*(\d+)\s+(\d+)\s*$/.exec(output ?? "");
  return found ? { ahead: Number(found[1]), behind: Number(found[2]) } : null;
}

/** Did `git push --dry-run` fail because of who we are (not because of the network or a stale branch)? */
export function pushWasDenied(output) {
  return /authentication failed|could not read username|permission to .* denied|403|terminal prompts disabled|invalid credentials/i.test(
    output ?? "",
  );
}

/** What each check needs on each system. `{pnpm}` is replaced by the version the repo pins. */
export const FIXES = {
  clone: {
    all: "git clone https://github.com/userAshwani/lunaRun.git   (a copy made without its .git folder cannot be updated: work in the clone)",
  },
  git: {
    win32: "winget install --id Git.Git -e",
    darwin: "xcode-select --install   (or: brew install git)",
    linux: "sudo apt install git   (or your distribution's package manager)",
  },
  node: {
    win32: "winget install --id OpenJS.NodeJS.LTS -e   (then open a new terminal)",
    darwin: "brew install node@24",
    linux: "install Node 24 LTS from https://nodejs.org or with nvm",
  },
  pnpm: {
    all: "corepack enable && corepack prepare pnpm@{pnpm} --activate   (or: npm install -g pnpm@{pnpm})",
  },
  rust: {
    win32: "winget install --id Rustlang.Rustup -e   (then open a new terminal)",
    darwin: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
    linux: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
  },
  rustComponents: { all: "rustup component add rustfmt clippy" },
  msvc: {
    win32:
      'winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"   (asks for administrator approval; if the id is gone, `winget search "Visual Studio Build Tools"`)',
  },
  webview2: { win32: "winget install --id Microsoft.EdgeWebView2Runtime -e" },
  xcode: {
    darwin: "xcode-select --install   (then accept the licence: sudo xcodebuild -license accept)",
  },
  linuxLibs: {
    linux:
      "sudo apt install build-essential curl wget file pkg-config libssl-dev libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev   (Debian and Ubuntu; other distributions: https://v2.tauri.app/start/prerequisites/)",
  },
  identity: {
    all: 'git config user.name "<name>" && git config user.email "<email>"   (use the author of the existing commits: git log -3 --format="%an <%ae>")',
  },
  push: {
    all: "gh auth login --web --git-protocol https && gh auth setup-git   (the owner finishes the browser step once; never paste a token into the chat)",
  },
  gh: {
    win32: "winget install --id GitHub.cli -e",
    darwin: "brew install gh",
    linux: "see https://github.com/cli/cli#installation",
  },
  deps: { all: "pnpm install --frozen-lockfile" },
  pull: { all: "git pull --ff-only" },
};

/** The fix text for `id` on `platform`, or a pointer to the docs when this system has none. */
export function fixFor(id, platform, pnpm = "12.16.1") {
  const table = FIXES[id] ?? {};
  const text = table[platform] ?? table.all ?? "see https://v2.tauri.app/start/prerequisites/";
  return text.replaceAll("{pnpm}", pnpm);
}

/** One printable line per result, and the verdict. `results`: `{ state: "ok" | "missing" | "warn", name, detail, fix }`. */
export function renderReport(results, heading) {
  const label = { ok: "ok     ", warn: "warn   ", missing: "MISSING" };
  const lines = [heading, ""];
  for (const r of results) {
    lines.push(`  ${label[r.state]}  ${r.name}${r.detail ? `: ${r.detail}` : ""}`);
    if (r.state !== "ok" && r.fix) lines.push(`           fix: ${r.fix}`);
  }
  const missing = results.filter((r) => r.state === "missing");
  lines.push("");
  lines.push(
    missing.length === 0
      ? "READY: everything needed here is in place."
      : `NOT READY: ${missing.length} required item${missing.length === 1 ? "" : "s"} missing (${missing
          .map((r) => r.name)
          .join(", ")}).`,
  );
  return { text: lines.join("\n"), ready: missing.length === 0 };
}

// ---------------------------------------------------------------------------------------------
// Looking at this computer
// ---------------------------------------------------------------------------------------------

/** Run a program and give back `{ ok, out }` (stdout and stderr together). Never throws, never opens a prompt. */
function run(program, args, options = {}) {
  // pnpm and npm are .cmd files on Windows and need a shell; every argument here is a constant.
  const shell = process.platform === "win32" && ["pnpm", "npm", "corepack"].includes(program);
  const result = spawnSync(program, args, {
    cwd: options.cwd ?? ROOT,
    encoding: "utf8",
    shell,
    timeout: options.timeout ?? 60_000,
    env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
  });
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  return { ok: result.status === 0 && !result.error, out };
}

function readOptional(file) {
  try {
    return readFileSync(path.join(ROOT, file), "utf8");
  } catch {
    return "";
  }
}

export function checkMachine(platform = process.platform) {
  const results = [];
  const add = (state, name, detail, fixId, pnpm) =>
    results.push({
      state,
      name,
      detail,
      fix: state === "ok" || !fixId ? "" : fixFor(fixId, platform, pnpm),
    });
  const pnpmWanted = parsePnpmVersion(readOptional("package.json")) ?? "12.16.1";
  const rustWanted = parseRustVersion(readOptional("Cargo.toml")) ?? "1.99";

  // --- tools ---
  const git = run("git", ["--version"]);
  add(git.ok ? "ok" : "missing", "git", git.ok ? versionIn(git.out) : "not found", "git");

  const nodeVersion = process.versions.node;
  const nodeOk = Number.parseInt(nodeVersion, 10) >= MIN_NODE_MAJOR;
  add(
    nodeOk ? "ok" : "missing",
    "Node.js",
    `${nodeVersion} (needs ${MIN_NODE_MAJOR} or newer)`,
    "node",
  );

  const pnpm = run("pnpm", ["--version"]);
  if (!pnpm.ok) add("missing", "pnpm", "not found", "pnpm", pnpmWanted);
  else if (versionIn(pnpm.out) !== pnpmWanted)
    add("warn", "pnpm", `${versionIn(pnpm.out)}, the repo pins ${pnpmWanted}`, "pnpm", pnpmWanted);
  else add("ok", "pnpm", pnpmWanted);

  // `rustc` inside the repo makes rustup fetch the toolchain that rust-toolchain.toml pins, which can take a minute.
  const rustc = run("rustc", ["--version"], { timeout: 600_000 });
  const rustVersion = versionIn(rustc.out);
  if (!rustc.ok || !rustVersion) add("missing", "Rust", "not found", "rust");
  else if (compareVersions(rustVersion, rustWanted) < 0)
    add("missing", "Rust", `${rustVersion}, needs ${rustWanted} or newer`, "rust");
  else add("ok", "Rust", `${rustVersion} (needs ${rustWanted} or newer)`);

  const components = ["fmt", "clippy"].map((tool) => run("cargo", [tool, "--version"]));
  add(
    components.every((c) => c.ok) ? "ok" : "missing",
    "rustfmt and clippy",
    components.every((c) => c.ok) ? "" : "not installed",
    "rustComponents",
  );

  // --- what Tauri needs on this system ---
  if (platform === "win32") {
    const vswhere = path.join(
      process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
      "Microsoft Visual Studio",
      "Installer",
      "vswhere.exe",
    );
    const tools = existsSync(vswhere)
      ? run(vswhere, [
          "-latest",
          "-products",
          "*",
          "-requires",
          "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
          "-property",
          "installationPath",
        ])
      : { ok: false, out: "" };
    add(
      tools.ok && tools.out ? "ok" : "missing",
      "Visual Studio C++ build tools",
      tools.ok && tools.out ? "" : "not found (Rust cannot link without them)",
      "msvc",
    );
    const key = String.raw`SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`;
    const webview =
      run("reg", ["query", `HKLM\\${key}`, "/v", "pv"]).ok ||
      run("reg", ["query", `HKCU\\${key}`, "/v", "pv"]).ok;
    add(webview ? "ok" : "missing", "WebView2 runtime", webview ? "" : "not found", "webview2");
  } else if (platform === "darwin") {
    const xcode = run("xcode-select", ["-p"]);
    add(
      xcode.ok ? "ok" : "missing",
      "Xcode command line tools",
      xcode.ok ? xcode.out : "not found",
      "xcode",
    );
  } else {
    const pkg = run("pkg-config", ["--version"]);
    const libs = pkg.ok
      ? run("pkg-config", ["--exists", "webkit2gtk-4.1", "openssl", "librsvg-2.0"])
      : { ok: false };
    add(
      libs.ok ? "ok" : "missing",
      "Tauri system libraries",
      libs.ok
        ? ""
        : pkg.ok
          ? "webkit2gtk-4.1, openssl or librsvg not found"
          : "pkg-config not found",
      "linuxLibs",
    );
  }

  // --- the repository ---
  const inside = run("git", ["rev-parse", "--is-inside-work-tree"]);
  if (!inside.ok) {
    add("missing", "repository", "this folder is not a git clone", "clone");
  } else {
    const remote = run("git", ["remote", "get-url", "origin"]);
    add(
      remote.ok && REMOTE_PATTERN.test(remote.out) ? "ok" : "missing",
      "repository",
      remote.ok ? `origin is ${remote.out}` : "no remote named origin",
      "clone",
    );

    const fetched = run("git", ["fetch", "--quiet", "origin"], { timeout: 120_000 });
    if (!fetched.ok) {
      add(
        "warn",
        "up to date with GitHub",
        "could not reach GitHub (offline, or no access)",
        "push",
      );
    } else {
      const counts = parseAheadBehind(
        run("git", ["rev-list", "--left-right", "--count", "HEAD...origin/main"]).out,
      );
      const dirty = run("git", ["status", "--porcelain"]).out.length > 0;
      if (counts && counts.behind > 0)
        add(
          "warn",
          "up to date with GitHub",
          `${counts.behind} commit(s) behind origin/main`,
          "pull",
        );
      else if (counts && counts.ahead > 0)
        add("warn", "up to date with GitHub", `${counts.ahead} commit(s) not pushed yet`, "");
      else
        add(
          "ok",
          "up to date with GitHub",
          dirty
            ? "level with origin/main, but there are uncommitted changes"
            : "level with origin/main",
        );
    }

    const name = run("git", ["config", "user.name"]);
    const email = run("git", ["config", "user.email"]);
    add(
      name.ok && email.ok && name.out && email.out ? "ok" : "missing",
      "git identity",
      name.ok && email.ok ? `${name.out} <${email.out}>` : "user.name or user.email is not set",
      "identity",
    );

    // A dry run authenticates like a real push and sends nothing.
    const push = run("git", ["push", "--dry-run", "origin", "HEAD:refs/heads/main"], {
      timeout: 120_000,
    });
    if (push.ok || (!pushWasDenied(push.out) && /rejected|up-to-date|->/.test(push.out)))
      add("ok", "permission to push", "");
    else if (pushWasDenied(push.out))
      add("missing", "permission to push", "GitHub did not accept this computer's login", "push");
    else add("warn", "permission to push", "could not tell (offline?)", "push");

    const gh = run("gh", ["auth", "status"]);
    const ghInstalled = run("gh", ["--version"]).ok;
    if (!ghInstalled)
      add("warn", "GitHub CLI (gh)", "not installed: needed to watch CI runs", "gh");
    else if (!gh.ok) add("warn", "GitHub CLI (gh)", "installed, not logged in", "push");
    else add("ok", "GitHub CLI (gh)", "logged in");
  }

  // --- dependencies ---
  add(
    existsSync(path.join(ROOT, "node_modules")) ? "ok" : "missing",
    "front-end dependencies",
    existsSync(path.join(ROOT, "node_modules"))
      ? "node_modules is there"
      : "node_modules is missing",
    "deps",
  );

  const names = { win32: "Windows", darwin: "macOS", linux: "Linux" };
  return renderReport(
    results,
    `lunaRun machine check (${names[platform] ?? platform}, Node ${nodeVersion})`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { text, ready } = checkMachine();
  console.log(text);
  process.exit(ready ? 0 : 1);
}
