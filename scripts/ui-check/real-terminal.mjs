// The terminal of a project, through the REAL Tauri bridge of the real app window (FR-PJ-08, D-040). Windows only.
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-terminal.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with WebView2's remote debugging port open and a throw-away data folder (`<scratch>/lüna home`),
// adds a project (`<scratch>/mon projet é`), and then:
//   1. asks for a terminal while no PHP is installed: it must be refused with NO_SITE (reason `php`), and nothing may be
//      written into the data folder (no shim, no script);
//   2. installs the newest PHP and Composer from the Runtimes commands (about 35 MB from the internet), checks the page
//      offers "Open terminal", and asks for a terminal: a real PowerShell window must open (found by the banner it was
//      started with), the Composer shim must exist inside the data folder and name its programs relative to itself;
//   3. closes that window again, so nothing is left behind.
// The folder names have a space and a non-ASCII letter (NFR-07). It ends the app.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-terminal.mjs <lunarun.exe> <scratch folder>");
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
const project = path.join(path.resolve(scratch), "mon projet é");
fs.rmSync(home, { recursive: true, force: true });
fs.rmSync(project, { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });
fs.mkdirSync(path.join(project, "public"), { recursive: true });
fs.writeFileSync(path.join(project, "public", "index.php"), "<?php\n");
fs.writeFileSync(path.join(project, "artisan"), "");
fs.writeFileSync(
  path.join(project, "composer.json"),
  JSON.stringify({ require: { php: ">=8.1", "laravel/framework": "^11.0" } }),
);

const app = spawn(path.resolve(exe), [], {
  env: {
    ...process.env,
    LUNA_HOME: home,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
  },
  stdio: "ignore",
});
const stop = () =>
  spawnSync("taskkill", ["/PID", String(app.pid), "/T", "/F"], { stdio: "ignore" });
process.on("exit", stop);

async function pageSocket() {
  for (let attempt = 0; attempt < 120; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
      const page = targets.find((t) => t.type === "page" && !t.url.startsWith("devtools:"));
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("the app's window never opened its debugging port");
}

const socket = new WebSocket(await pageSocket());
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = reject;
});
let nextId = 1;
const waiting = new Map();
socket.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && waiting.has(message.id)) waiting.get(message.id)(message);
};
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const id = nextId++;
    waiting.set(id, resolve);
    socket.send(JSON.stringify({ id, method, params }));
  });
const evaluate = (expression) =>
  send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });

async function waitForText(text, seconds = 60) {
  for (let attempt = 0; attempt < seconds * 2; attempt++) {
    const reply = await evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`);
    if (reply.result?.result?.value === true) return true;
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

async function invoke(command, args = {}) {
  const reply = await evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})
       .then((value) => ({ ok: value }), (err) => ({ err }))`,
  );
  assert.ok(reply.result?.result, `no answer from ${command}: ${JSON.stringify(reply)}`);
  return reply.result.result.value;
}

async function screenshot(name) {
  const reply = await send("Page.captureScreenshot", { format: "png" });
  const file = path.join(path.resolve(scratch), `${name}.png`);
  fs.writeFileSync(file, Buffer.from(reply.result.data, "base64"));
  return file;
}

for (let attempt = 0; attempt < 60; attempt++) {
  const probe = await evaluate("typeof window.__TAURI_INTERNALS__");
  if (probe.result?.result?.value === "object") break;
  await new Promise((r) => setTimeout(r, 500));
}

const results = [];
const check = (name, fn) => {
  try {
    fn();
    results.push(`ok    ${name}`);
  } catch (error) {
    results.push(`FAIL  ${name}: ${error.message}`);
    process.exitCode = 1;
  }
};

async function finished(jobId, seconds) {
  for (let attempt = 0; attempt < seconds * 4; attempt++) {
    const jobs = await invoke("jobs_list");
    const found = jobs.ok?.find((job) => job.id === jobId);
    if (found && found.state !== "running") return found;
    await new Promise((r) => setTimeout(r, 250));
  }
  return null;
}

/** The PowerShell windows lunaRun opened: the banner is in their command line. */
function terminalProcesses() {
  const script =
    "Get-CimInstance Win32_Process -Filter \"Name = 'powershell.exe'\" | " +
    // The query's own command line contains the banner text, so it must not count itself.
    "Where-Object { $_.CommandLine -like '*Write-Host ''lunaRun: PHP*' -and $_.CommandLine -notlike '*Get-CimInstance*' } | " +
    'ForEach-Object { "$($_.ProcessId)|$($_.CommandLine)" }';
  const out = spawnSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" });
  return out.stdout
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => {
      const [pid, ...rest] = line.split("|");
      return { pid: Number(pid), commandLine: rest.join("|") };
    });
}

const added = await invoke("project_add", { path: project });
const id = added.ok?.project?.id;
check("the project is added", () => assert.ok(id, JSON.stringify(added)));

// 1. No PHP: refused, and nothing is written.
const refused = await invoke("project_open", { id, target: "terminal" });
check("without PHP a terminal is refused with NO_SITE (reason php)", () => {
  assert.equal(refused.err?.code, "NO_SITE", JSON.stringify(refused));
  assert.equal(refused.err.details.reason, "php");
});
check("and nothing is written for it", () => {
  assert.ok(!fs.existsSync(path.join(home, "run", "terminals")));
  assert.ok(!fs.existsSync(path.join(home, "config", "shims")));
});
const unknown = await invoke("project_open", { id: "nope", target: "terminal" });
check("a terminal for an unknown project is NOT_FOUND", () =>
  assert.equal(unknown.err?.code, "NOT_FOUND"),
);

// 2. Install the newest PHP and Composer that have a build for this computer.
const available = await invoke("runtimes_available");
assert.ok(available.ok, JSON.stringify(available));
const newest = (runtime) =>
  available.ok.runtimes
    .find((r) => r.id === runtime)
    ?.versions.find((v) => v.channel !== "prerelease" && !v.eol)?.version;
for (const runtime of ["php", "composer"]) {
  const version = newest(runtime);
  assert.ok(version, `no ${runtime} version to install`);
  const started = await invoke("runtime_install", { runtime, version });
  assert.ok(started.ok, JSON.stringify(started));
  const job = await finished(started.ok.id, 300);
  check(`${runtime} ${version} installs`, () =>
    assert.equal(job?.state, "succeeded", JSON.stringify(job)),
  );
}

await evaluate(`location.hash = '#/projects/${id}'`);
const shown = await waitForText("Open terminal");
check("the project page shows Open terminal in the real window", () => assert.ok(shown));
console.log("screenshot:", await screenshot("real-terminal-page"));

const before = new Set(terminalProcesses().map((p) => p.pid));
const opened = await invoke("project_open", { id, target: "terminal" });
check("with PHP installed the terminal command succeeds", () =>
  assert.deepEqual(opened, { ok: null }, JSON.stringify(opened)),
);
let windows = [];
for (let attempt = 0; attempt < 40 && windows.length === 0; attempt++) {
  windows = terminalProcesses().filter((p) => !before.has(p.pid));
  if (windows.length === 0) await new Promise((r) => setTimeout(r, 250));
}
check("a real PowerShell window opened with the project's versions in its banner", () => {
  assert.equal(windows.length, 1, JSON.stringify(windows));
  assert.match(
    windows[0].commandLine,
    /Write-Host 'lunaRun: PHP \d+\.\d+\.\d+(, Composer \d+\.\d+\.\d+)?'/,
  );
});
const shim = path.join(home, "config", "shims");
const shimFiles = fs.existsSync(shim)
  ? fs
      .readdirSync(shim)
      .flatMap((d) => fs.readdirSync(path.join(shim, d)).map((f) => path.join(shim, d, f)))
  : [];
check("the Composer shim is inside the data folder and has no accented letter in it", () => {
  assert.equal(shimFiles.length, 1, JSON.stringify(shimFiles));
  const text = fs.readFileSync(shimFiles[0], "utf8");
  assert.match(text, /%~dp0\.\.\\\.\.\\\.\.\\runtimes\\php\\/);
  assert.ok(/^[\x00-\x7f]*$/.test(text), text);
});

// 3. Close the window again.
for (const w of windows)
  spawnSync("taskkill", ["/PID", String(w.pid), "/T", "/F"], { stdio: "ignore" });
await new Promise((r) => setTimeout(r, 500));
check("and it is closed again", () =>
  assert.equal(terminalProcesses().filter((p) => !before.has(p.pid)).length, 0),
);

console.log(results.join("\n"));
socket.close();
stop();
