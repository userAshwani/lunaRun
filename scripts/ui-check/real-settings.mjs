// The Settings page's backend and launch at login (FR-AP-03, D-045) in the REAL app. Windows only.
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-settings.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with a throw-away data folder (`<scratch>/lüna home`) and proves:
//   1. the four commands are permitted, a setting is stored in the data folder and a refused one writes nothing;
//   2. launch at login really reaches Windows: switched on, the Run key HKCU\...\CurrentVersion\Run has an entry that
//      starts this program with `--hidden`; switched off, the entry is gone. If launch at login was already on before the
//      check (the owner's own choice), this part is skipped and nothing is changed. It is always switched off at the end;
//   3. started with `--hidden`, lunaRun runs with no visible window (it waits in the tray), and started without it, the
//      window shows.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-settings.mjs <lunarun.exe> <scratch folder>");
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
fs.rmSync(home, { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });

function powershell(script) {
  return spawnSync("powershell", ["-NoProfile", "-Command", script], {
    encoding: "utf8",
  }).stdout.trim();
}
const RUN_KEY = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/** The Run entries that start this program, as `name=command`. */
const runEntries = () =>
  powershell(
    `$p = Get-ItemProperty '${RUN_KEY}'; $p.PSObject.Properties | Where-Object { "$($_.Value)" -like '*lunarun.exe*' } | ForEach-Object { "$($_.Name)=$($_.Value)" }`,
  )
    .split(/\r?\n/)
    .filter(Boolean);

function start(args = []) {
  return spawn(path.resolve(exe), args, {
    env: {
      ...process.env,
      LUNA_HOME: home,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
    },
    stdio: "ignore",
  });
}
const end = (child) =>
  spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });

// The windows of a process that Windows marks visible, listed from C# (text written from a PowerShell callback is lost,
// which made an earlier version of this list always empty). The windowing library keeps one internal window per app,
// class `Tao Thread Event Target`, 14 x 14 pixels and never on screen: it is not a window the user sees, so it is left out.
const WINDOW_LIST = `
param([int]$TargetPid)
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class W3 {
  public delegate bool Cb(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(Cb cb, IntPtr l);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  public static List<string> Visible(uint pid) {
    var found = new List<string>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) {
        var t = new StringBuilder(256); GetWindowText(h, t, 256);
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        if (c.ToString() != "Tao Thread Event Target") found.Add("title='" + t + "' class='" + c + "'");
      }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@
[W3]::Visible([uint32]$TargetPid)
`;
const windowListScript = path.join(path.resolve(scratch), "visible-windows.ps1");
fs.writeFileSync(windowListScript, WINDOW_LIST);

/** The windows of this process the user could see, as `title=... class=...` lines (empty: none). */
function visibleWindows(pid) {
  return spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      windowListScript,
      "-TargetPid",
      String(pid),
    ],
    { encoding: "utf8" },
  ).stdout.trim();
}

/** Is a window of this process visible? Waits up to `seconds` for one to appear. */
function visibleWindow(pid, seconds) {
  const until = Date.now() + seconds * 1000;
  do {
    if (visibleWindows(pid)) return true;
    spawnSync("powershell", ["-NoProfile", "-Command", "Start-Sleep -Milliseconds 250"]);
  } while (Date.now() < until);
  return false;
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

// 3a. Started as at login, before anything else ran (a lunaRun started right after another was killed can show a
// window of WebView2's for a moment while the old one shuts down: not a login situation, seen on 2026-11-28).
let app = start(["--hidden"]);
process.on("exit", () => end(app));
let seen = false;
for (let i = 0; i < 16 && !seen; i++) {
  await new Promise((r) => setTimeout(r, 500));
  seen = visibleWindow(app.pid, 0);
}
const alive =
  powershell(`if (Get-Process -Id ${String(app.pid)} -ErrorAction SilentlyContinue) { 'yes' }`) ===
  "yes";
check("started with --hidden, lunaRun runs", () => assert.ok(alive));
check("and shows no window for 8 seconds (it waits in the tray)", () =>
  assert.ok(!seen, visibleWindows(app.pid)),
);
end(app);
for (let attempt = 0; attempt < 40; attempt++) {
  if (
    powershell(`@(Get-Process -Id ${String(app.pid)} -ErrorAction SilentlyContinue).Count`) === "0"
  )
    break;
  await new Promise((r) => setTimeout(r, 250));
}
await new Promise((r) => setTimeout(r, 4000));

// 3b. Started normally, the window shows.
app = start();

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

const normal = visibleWindow(app.pid, 20) ? visibleWindows(app.pid) : "";
check("started normally, the lunaRun window is visible", () =>
  assert.ok(normal.includes("title='lunaRun'"), normal || "(no visible window)"),
);

// 1. The settings.
const view = await invoke("settings_get");
check("settings_get is permitted and starts at the defaults", () => {
  assert.ok(view.ok, JSON.stringify(view));
  assert.deepEqual(view.ok.current, view.ok.defaults);
});
const saved = await invoke("settings_set", {
  settings: { ...view.ok.current, defaultPhpLine: "8.4" },
});
check("settings_set is permitted and stores a setting in the data folder", () => {
  assert.equal(saved.ok?.current.defaultPhpLine, "8.4", JSON.stringify(saved));
  const file = JSON.parse(fs.readFileSync(path.join(home, "config", "settings.json"), "utf8"));
  assert.equal(file.defaultPhpLine, "8.4");
  assert.equal(file.schema, 1);
});
const refused = await invoke("settings_set", {
  settings: { ...saved.ok.current, registryUrl: "http://plain.example/r.json" },
});
check("a refused value names its field and writes nothing", () => {
  assert.equal(refused.err?.code, "INVALID_URL", JSON.stringify(refused));
  assert.equal(refused.err.details.field, "registryUrl");
  const file = JSON.parse(fs.readFileSync(path.join(home, "config", "settings.json"), "utf8"));
  assert.equal(file.registryUrl, undefined);
});

// 2. Launch at login, for real, unless the owner already uses it.
const before = await invoke("launch_at_login_get");
check("launch_at_login_get is permitted", () =>
  assert.equal(typeof before.ok, "boolean", JSON.stringify(before)),
);
if (before.ok === true) {
  results.push("skip  launch at login was already on (the owner's own setting): left as it is");
} else {
  try {
    const on = await invoke("launch_at_login_set", { enabled: true });
    const entries = runEntries();
    check("switched on, Windows starts this program at login, with --hidden", () => {
      assert.equal(on.ok, true, JSON.stringify(on));
      assert.equal(entries.length, 1, JSON.stringify(entries));
      assert.ok(entries[0].toLowerCase().includes(path.resolve(exe).toLowerCase()), entries[0]);
      assert.ok(entries[0].includes("--hidden"), entries[0]);
    });
  } finally {
    const off = await invoke("launch_at_login_set", { enabled: false });
    check("switched off, the entry is gone", () => {
      assert.equal(off.ok, false, JSON.stringify(off));
      assert.deepEqual(runEntries(), []);
    });
  }
}
socket.close();
end(app);

console.log(results.join("\n"));
