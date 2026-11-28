// The Diagnostics page and its report (FR-AP-04, D-046) in the REAL app window. Windows only (WebView2).
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-diagnostics.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with a throw-away data folder (`<scratch>/lüna home`, which lies under the user's profile when
// the scratch folder does) and a custom version-list address with a planted token, and proves that `diagnostics_get` is
// permitted, that the report's text names the common ports with what holds them (on a machine running XAMPP: 3306 taken
// by mysqld.exe), that the user's home folder is written as `~` and the token never appears, and that the real page shows it.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-diagnostics.mjs <lunarun.exe> <scratch folder>");
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
fs.rmSync(home, { recursive: true, force: true });
fs.mkdirSync(path.join(home, "config"), { recursive: true });
const TOKEN = "planted-token-9f8e7d";
fs.writeFileSync(
  path.join(home, "config", "settings.json"),
  JSON.stringify({ schema: 1, registryUrl: `https://mirror.example.org/r.json?token=${TOKEN}` }),
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
async function invoke(command, args = {}) {
  const reply = await evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})
       .then((value) => ({ ok: value }), (err) => ({ err }))`,
  );
  assert.ok(reply.result?.result, `no answer from ${command}: ${JSON.stringify(reply)}`);
  return reply.result.result.value;
}
async function waitForText(text, seconds = 60) {
  for (let attempt = 0; attempt < seconds * 2; attempt++) {
    const reply = await evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`);
    if (reply.result?.result?.value === true) return true;
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
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

const answer = await invoke("diagnostics_get");
check("diagnostics_get is permitted", () => assert.ok(answer.ok, JSON.stringify(answer)));
const text = answer.ok?.text ?? "";
console.log("---- the report ----\n" + text + "--------------------");
check("the report names the common ports", () => {
  for (const port of [80, 443, 3306, 8080])
    assert.match(text, new RegExp(`^- ${String(port)}`, "m"));
});
const xampp = spawnSync(
  "powershell",
  [
    "-NoProfile",
    "-Command",
    "@(Get-Process mysqld -ErrorAction SilentlyContinue | Where-Object { $_.Path -like '*xampp*' }).Count",
  ],
  { encoding: "utf8" },
).stdout.trim();
if (xampp !== "0") {
  check("XAMPP's MySQL is named as the holder of 3306", () =>
    assert.match(text, /^- 3306: TAKEN by mysqld\.exe/m),
  );
} else {
  results.push("skip  XAMPP's MySQL is not running here: 3306 is not checked against it");
}
const userHome = os.homedir();
check("the user's home folder is written as ~", () => {
  assert.ok(!text.toLowerCase().includes(userHome.toLowerCase()), text);
  if (home.toLowerCase().startsWith(userHome.toLowerCase())) assert.match(text, /Data folder: ~\\/);
});
check("the planted token is nowhere, the custom host is", () => {
  assert.ok(!text.includes(TOKEN), text);
  assert.ok(!JSON.stringify(answer).includes(TOKEN), "not in the page's data either");
  assert.match(text, /custom address \(mirror\.example\.org\)/);
});

// A fresh data folder opens on the first-run setup (D-044): finish it, as a user would.
await invoke("onboarding_complete");
await evaluate("location.hash = '#/diagnostics'");
await evaluate("location.reload()");
await new Promise((r) => setTimeout(r, 2000));
const shown = await waitForText("Often used by other programs");
check("the real page shows the ports", () => assert.ok(shown));
const body = (await evaluate("document.body.innerText")).result?.result?.value ?? "";
check("with the copy button and the note on what is left out", () => {
  assert.ok(body.includes("Copy report"), body);
  assert.ok(body.includes("Check again"), body);
  assert.ok(body.includes("leaves out your user name"), body);
  assert.ok(!body.includes(TOKEN), "the token is not on the page");
});
console.log("screenshot:", await screenshot("real-diagnostics"));

console.log(results.join("\n"));
socket.close();
stop();
