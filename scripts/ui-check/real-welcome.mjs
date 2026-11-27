// The first-run wizard (FR-AP-06, D-044) in the REAL app window. Windows only (WebView2).
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-welcome.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with an empty throw-away data folder (`<scratch>/lüna home`, given through `LUNA_HOME`) and proves:
//   1. the app opens on the wizard, and its five commands are permitted (capabilities/default.json);
//   2. the data folder is the one from the environment, and choosing another is refused (`fromEnv`), so this check never
//      writes the real pointer file of this computer (it checks that it did not);
//   3. a folder with other files in it is refused by the check, and nothing is written into it;
//   4. in the real page, with only Composer chosen (about 3 MB), Install starts one ordinary install job that succeeds,
//      and "Go to the dashboard" finishes the wizard: it is not the first run any more.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-welcome.mjs <lunarun.exe> <scratch folder>");
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
const busy = path.join(path.resolve(scratch), "my work");
fs.rmSync(home, { recursive: true, force: true });
fs.rmSync(busy, { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });
fs.mkdirSync(busy, { recursive: true });
fs.writeFileSync(path.join(busy, "notes.txt"), "mine");
const pointer = path.join(process.env.APPDATA ?? "", "lunaRun-home.json");
const pointerBefore = fs.existsSync(pointer) ? fs.readFileSync(pointer, "utf8") : null;

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
/** Click the button (or the checkbox's label) whose text is `label`. */
async function click(label, selector = "button") {
  const reply = await evaluate(`(() => {
    const el = [...document.querySelectorAll(${JSON.stringify(selector)})].find((b) => b.innerText.trim() === ${JSON.stringify(label)});
    if (!el) return false;
    el.click();
    return true;
  })()`);
  return reply.result?.result?.value === true;
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

// 1. The app opens on the wizard.
const opened = await waitForText("Welcome to lunaRun");
check("the app opens on the first-run wizard", () => assert.ok(opened));
const hash = (await evaluate("location.hash")).result?.result?.value;
check("at #/welcome", () => assert.equal(hash, "#/welcome"));
console.log("screenshot:", await screenshot("real-welcome-home"));

const view = await invoke("onboarding_get");
check("onboarding_get is permitted and says it is the first run", () => {
  assert.ok(view.ok, JSON.stringify(view));
  assert.equal(view.ok.show, true);
  assert.equal(view.ok.pack.items.length + view.ok.pack.unavailable.length, 4);
});

// 2. The environment sets the folder: it cannot be chosen, and the real pointer is not written.
check("the data folder is the environment's, and cannot be changed here", () => {
  assert.equal(view.ok.homeSource, "env");
  assert.equal(view.ok.canChooseHome, false);
  assert.equal(path.resolve(view.ok.home), path.resolve(home));
});
const elsewhere = path.join(path.resolve(scratch), "elsewhere");
const refused = await invoke("onboarding_choose_home", { path: elsewhere });
check("onboarding_choose_home is permitted and refuses (fromEnv)", () => {
  assert.equal(refused.err?.code, "INVALID_INPUT", JSON.stringify(refused));
  assert.equal(refused.err.details.reason, "fromEnv");
  assert.ok(!fs.existsSync(elsewhere), "nothing was made");
});
const pointerAfter = fs.existsSync(pointer) ? fs.readFileSync(pointer, "utf8") : null;
check("the real pointer file of this computer is untouched", () =>
  assert.equal(pointerAfter, pointerBefore),
);

// 3. A folder with other files is refused, and nothing is written into it.
const busyCheck = await invoke("onboarding_check_home", { path: busy });
check("onboarding_check_home is permitted and refuses a folder with other files", () => {
  assert.equal(busyCheck.err?.code, "INVALID_INPUT", JSON.stringify(busyCheck));
  assert.equal(busyCheck.err.details.reason, "notEmpty");
  assert.deepEqual(fs.readdirSync(busy), ["notes.txt"]);
});
const freshCheck = await invoke("onboarding_check_home", { path: elsewhere });
check("and accepts a new folder without making it", () => {
  assert.deepEqual(freshCheck.ok?.warnings, []);
  assert.ok(!fs.existsSync(elsewhere));
});

// 4. Through the real page: only Composer, installed for real; then finish.
assert.ok(await click("Continue"), "Continue (1)");
await waitForText("This computer");
assert.ok(await click("Continue"), "Continue (2)");
await waitForText("Laravel starter pack");
for (const item of view.ok.pack.items.filter((i) => i.runtime !== "composer")) {
  const name = { php: "PHP", node: "Node.js", mysql: "MySQL" }[item.runtime] ?? item.runtime;
  assert.ok(await click(`${name} ${item.version}`, "label"), `uncheck ${item.runtime}`);
}
const oneShown = await waitForText("Install 1 runtime", 10);
check("with only Composer chosen the page offers one install", () => assert.ok(oneShown));
assert.ok(await click("Install 1 runtime"), "Install 1 runtime");
const composer = view.ok.pack.items.find((i) => i.runtime === "composer");
let job = null;
for (let attempt = 0; attempt < 240 && !job; attempt++) {
  const jobs = await invoke("jobs_list");
  const found = jobs.ok?.find((j) => j.key === `composer@${composer.version}`);
  if (found && found.state !== "running") job = found;
  else await new Promise((r) => setTimeout(r, 500));
}
check("the Composer install succeeded", () =>
  assert.equal(job?.state, "succeeded", JSON.stringify(job)),
);
const jobs = await invoke("jobs_list");
check("and nothing else was installed", () =>
  assert.deepEqual(
    jobs.ok.map((j) => j.key.split("@")[0]),
    ["composer"],
  ),
);
await waitForText("Installed");
console.log("screenshot:", await screenshot("real-welcome-pack"));
assert.ok(await click("Go to the dashboard"), "Go to the dashboard");
await new Promise((r) => setTimeout(r, 1500));
const after = await invoke("onboarding_get");
check("finishing ends the first run", () => assert.equal(after.ok?.show, false));
const hashAfter = (await evaluate("location.hash")).result?.result?.value;
check("and opens the dashboard", () =>
  assert.ok(hashAfter === "#/" || hashAfter === "", hashAfter),
);
check("the wizard's state is in the data folder", () =>
  assert.ok(fs.existsSync(path.join(home, "config", "onboarding.json"))),
);

console.log(results.join("\n"));
socket.close();
stop();
