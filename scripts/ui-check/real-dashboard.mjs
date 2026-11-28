// The Dashboard (D-047) in the REAL app window. Windows only (WebView2).
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-dashboard.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with a throw-away data folder (`<scratch>/lüna home`), proves that the five commands the
// Dashboard reads are permitted, adds a project (a folder with a space and a non-ASCII letter holding only a
// composer.json) and checks the real page: the project's row with Start disabled because no PHP is installed, the
// ticked "Add a project" step, nothing running and "No problems".
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-dashboard.mjs <lunarun.exe> <scratch folder>");
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
fs.rmSync(home, { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });
const projectDir = path.join(path.resolve(scratch), "my shop ü");
fs.rmSync(projectDir, { recursive: true, force: true });
fs.mkdirSync(projectDir, { recursive: true });
fs.writeFileSync(
  path.join(projectDir, "composer.json"),
  JSON.stringify({ name: "acme/shop", require: { php: "^8.3" } }),
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
       .then((value) => ({ ok: value ?? true }), (err) => ({ err }))`,
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

for (const command of [
  "services_list",
  "projects_list",
  "sites_list",
  "runtimes_installed",
  "jobs_list",
]) {
  const answer = await invoke(command);
  check(`${command} is permitted`, () => assert.ok("ok" in answer, JSON.stringify(answer)));
}
const added = await invoke("project_add", { path: projectDir });
check("the project is added", () => assert.ok("ok" in added, JSON.stringify(added)));

// A fresh data folder opens on the first-run setup (D-044): finish it, as a user would.
await invoke("onboarding_complete");
await evaluate("location.hash = '#/'");
await evaluate("location.reload()");
await new Promise((r) => setTimeout(r, 2000));
const shown = await waitForText("No problems");
const body = (await evaluate("document.body.innerText")).result?.result?.value ?? "";
check("the real page is the live Dashboard", () => {
  assert.ok(shown, body);
  assert.ok(body.includes("my shop ü"), body);
  assert.ok(body.includes("Nothing is running"), body);
  assert.ok(body.includes("Nothing installed yet"), body);
  assert.ok(body.includes("Get started"), "nothing is installed: the steps stay");
});
const start = await evaluate(
  `(() => { const b = document.querySelector('[aria-label="Start my shop ü"]'); return b ? b.disabled : null; })()`,
);
check("Start is there and disabled: no PHP is installed", () =>
  assert.equal(start.result?.result?.value, true),
);
const ticked = await evaluate(
  `[...document.querySelectorAll(".sr-only")].filter((e) => e.textContent === "Done").length`,
);
check("the step 'Add a project' is ticked", () => assert.equal(ticked.result?.result?.value, 1));
console.log("screenshot:", await screenshot("real-dashboard"));

console.log(results.join("\n"));
socket.close();
stop();
