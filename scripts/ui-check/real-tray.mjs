// The tray (FR-AP-01, FR-AP-02, D-043) in the REAL app window, with a REAL Caddy. Windows only.
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-tray.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with a throw-away data folder (`<scratch>/lüna home`), installs the newest Caddy (about 18 MB)
// through the app's own commands and starts it, then:
//   1. closes the window the way the title bar's X does (WM_CLOSE): the window must be hidden, the app must still run, and
//      Caddy must still run (closing no longer quits, FR-AP-01);
//   2. checks that Windows knows lunaRun's tray icon (Windows 11 lists every icon an app registered under
//      HKCU\Control Panel\NotifyIconSettings, with the program's path);
//   3. clicks the icon when UI Automation can see it (an icon Windows put into the hidden-icons area cannot be reached
//      this way: then this step says so and the owner checks it by hand), which must show the window again;
//   4. ends the app, and Caddy with it (its Job Object).
// Quitting from the tray's menu cannot be clicked from here reliably; the menu model is unit-tested (src-tauri/tests/tray.rs).
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-tray.mjs <lunarun.exe> <scratch folder>");
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
fs.rmSync(home, { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });

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

function powershell(script) {
  return spawnSync("powershell", ["-NoProfile", "-Command", script], {
    encoding: "utf8",
  }).stdout.trim();
}

const WIN32 = `
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
}
"@`;

/** The handle of lunaRun's main window: the visible top-level window of the app's process (its title is the page's). */
const handle = () =>
  powershell(
    `for ($i = 0; $i -lt 40; $i++) { $h = (Get-Process -Id ${String(app.pid)}).MainWindowHandle.ToInt64(); if ($h -ne 0) { $h; break }; Start-Sleep -Milliseconds 250 }`,
  ) || "0";
const visible = (h) => powershell(`${WIN32}; [W]::IsWindowVisible([IntPtr]${h})`) === "True";
const alive = (pid) =>
  powershell(
    `if (Get-Process -Id ${pid} -ErrorAction SilentlyContinue) { 'yes' } else { 'no' }`,
  ) === "yes";
const caddyRunning = () =>
  powershell(
    `@(Get-CimInstance Win32_Process -Filter "Name = 'caddy.exe'" | Where-Object { $_.ExecutablePath -like '*lunaRun*' -or $_.ExecutablePath -like '*na home*' }).Count`,
  ) !== "0";

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
async function invoke(command, args = {}) {
  const reply = await evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})
       .then((value) => ({ ok: value }), (err) => ({ err }))`,
  );
  assert.ok(reply.result?.result, `no answer from ${command}: ${JSON.stringify(reply)}`);
  return reply.result.result.value;
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

// A real Caddy, installed and started through the app.
const available = await invoke("runtimes_available");
const caddy = available.ok.runtimes
  .find((r) => r.id === "caddy")
  ?.versions.find((v) => v.channel !== "prerelease" && v.available)?.version;
const installed = await invoke("runtime_install", { runtime: "caddy", version: caddy });
const installJob = await finished(installed.ok?.id, 300);
check(`Caddy ${String(caddy)} installs`, () => assert.equal(installJob?.state, "succeeded"));
const started = await invoke("service_start", { id: "caddy" });
const startJob = await finished(started.ok?.id, 120);
check("Caddy starts", () => assert.equal(startJob?.state, "succeeded", JSON.stringify(startJob)));
check("and runs", () => assert.ok(caddyRunning()));

// 1. Close the window as its X does.
const h = handle();
check("the window is found and visible", () => {
  assert.ok(h !== "0", "no window called lunaRun");
  assert.ok(visible(h));
});
powershell(
  `${WIN32}; [W]::PostMessage([IntPtr]${h}, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null`,
);
await new Promise((r) => setTimeout(r, 2000));
check("closing hides the window", () => assert.ok(!visible(h)));
check("and lunaRun keeps running", () => assert.ok(alive(app.pid)));
check("and so does Caddy", () => assert.ok(caddyRunning()));

// 2. Windows knows the icon.
const known = powershell(
  `Get-ChildItem 'HKCU:\\Control Panel\\NotifyIconSettings' -ErrorAction SilentlyContinue | ForEach-Object { (Get-ItemProperty $_.PSPath).ExecutablePath } | Where-Object { $_ -like '*lunarun.exe' }`,
);
check("Windows has registered lunaRun's tray icon", () =>
  assert.ok(
    known.toLowerCase().includes("lunarun.exe"),
    `NotifyIconSettings: ${known || "(none)"}`,
  ),
);

// 3. Click the icon when UI Automation can reach it.
const clicked = powershell(`
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::RootElement
$all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
foreach ($e in $all) {
  try {
    if ($e.Current.Name -like 'lunaRun:*') {
      $p = $e.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
      $p.Invoke(); 'clicked'; break
    }
  } catch {}
}`);
if (clicked === "clicked") {
  await new Promise((r) => setTimeout(r, 1500));
  check("clicking the tray icon shows the window again", () => assert.ok(visible(h)));
} else {
  results.push(
    "skip  the tray icon is not reachable by UI Automation (Windows put it among the hidden icons): click it by hand",
  );
}

// 4. Ending the app ends Caddy with it.
socket.close();
stop();
await new Promise((r) => setTimeout(r, 2000));
check("ending lunaRun ends Caddy too", () => assert.ok(!caddyRunning()));
console.log(results.join("\n"));
