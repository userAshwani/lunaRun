// The fixes of the database row (`DB_PORT` in `.env` and Create database), through the REAL Tauri bridge and the REAL page of
// the real app window (FR-PJ-05, D-041). Windows only (WebView2).
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-env-fixes.mjs <path to lunarun.exe> <scratch folder>
//
// It starts the release app with WebView2's remote debugging port open and a throw-away data folder (`<scratch>/lüna home`)
// in which MySQL 8.4 is "installed" (a receipt and a stand-in program: nothing is downloaded and no server runs), adds a
// project whose `.env` has CRLF line endings, a wrong `DB_PORT` and a password, and then proves:
//   1. the four new commands are permitted by `capabilities/default.json` and answer as the page expects;
//   2. the preview shows the one line and writes nothing;
//   3. in the real page the button opens the dialog with the diff, Cancel writes nothing, and Update writes a backup first
//      and changes only that line (CRLF and the password line untouched);
//   4. a preview that is not the file's (forged, or the file edited meanwhile) is refused with ENV_CHANGED, nothing written;
//   5. with no server running the database cannot be checked or created, and the page says so.
// The folder names have a space and a non-ASCII letter (NFR-07). It ends the app.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error("usage: node scripts/ui-check/real-env-fixes.mjs <lunarun.exe> <scratch folder>");
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
const ORIGINAL =
  'APP_NAME=Shop\r\nDB_CONNECTION=mysql\r\nDB_HOST=127.0.0.1\r\nDB_PORT=1111 # old\r\nDB_DATABASE=shop\r\nDB_PASSWORD="hunter2 #1"\r\n';
fs.writeFileSync(path.join(project, ".env"), ORIGINAL);

// MySQL 8.4 "installed": a receipt and a stand-in program (as seed-home.mjs does).
const mysqlDir = path.join(home, "runtimes", "mysql", "8.4.13");
fs.mkdirSync(path.join(mysqlDir, "bin"), { recursive: true });
fs.writeFileSync(path.join(mysqlDir, "bin", "program"), "stand-in");
fs.writeFileSync(
  path.join(mysqlDir, "install.json"),
  JSON.stringify({
    schema: 1,
    runtime: "mysql",
    version: "8.4.13",
    os: "windows",
    arch: "x86_64",
    rosetta: false,
    installedAt: 1767225600,
    artifacts: [],
  }),
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

async function bodyText() {
  return (await evaluate("document.body.innerText")).result?.result?.value ?? "";
}

/** Click the button whose text is `label` (inside the dialog when one is open). */
async function clickButton(label) {
  const reply = await evaluate(`(() => {
    const scope = document.querySelector('[role="dialog"]') ?? document;
    const button = [...scope.querySelectorAll("button")].find((b) => b.innerText.trim() === ${JSON.stringify(label)});
    if (!button) return false;
    button.click();
    return true;
  })()`);
  return reply.result?.result?.value === true;
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
const envPath = path.join(project, ".env");
const read = () => fs.readFileSync(envPath, "utf8");
const backups = () => fs.readdirSync(project).filter((f) => f.startsWith(".env.lunarun-backup"));

const added = await invoke("project_add", { path: project });
const id = added.ok?.project?.id;
check("the project is added", () => assert.ok(id, JSON.stringify(added)));

// 1 and 2. The commands are permitted; the preview is the one line, and writes nothing.
const settings = await invoke("project_database", { id });
check("the database settings name the port lunaRun runs", () => {
  assert.ok(settings.ok, JSON.stringify(settings));
  assert.equal(settings.ok.runtime, "mysql");
  assert.equal(settings.ok.database, "shop");
});
const port = settings.ok?.port;
const preview = await invoke("project_env_port_preview", { id });
check("project_env_port_preview is permitted and shows the one line", () => {
  assert.ok(preview.ok, JSON.stringify(preview));
  assert.equal(preview.ok.port, port);
  assert.equal(preview.ok.line, 4);
  assert.equal(preview.ok.before, "DB_PORT=1111 # old");
  assert.equal(preview.ok.after, `DB_PORT=${String(port)} # old`);
  assert.equal(preview.ok.backup, ".env.lunarun-backup");
  assert.ok(!JSON.stringify(preview.ok).includes("hunter2"), "no other line, no password");
});
check("and writes nothing", () => {
  assert.equal(read(), ORIGINAL);
  assert.deepEqual(backups(), []);
});

// 4. A preview that is not the file's is refused.
const forged = await invoke("project_env_port_apply", {
  id,
  change: { ...preview.ok, after: "APP_NAME=evil" },
});
check("a forged preview is refused with ENV_CHANGED and nothing is written", () => {
  assert.equal(forged.err?.code, "ENV_CHANGED", JSON.stringify(forged));
  assert.equal(read(), ORIGINAL);
  assert.deepEqual(backups(), []);
});
const other = await invoke("project_env_port_apply", {
  id,
  change: { ...preview.ok, port: port + 1 },
});
check("so is one for a port that is not the one lunaRun runs", () => {
  assert.equal(other.err?.code, "ENV_CHANGED", JSON.stringify(other));
  assert.equal(read(), ORIGINAL);
});

// 3. In the real page.
await evaluate(`location.hash = '#/projects/${id}'`);
const shown = await waitForText("Update DB_PORT in .env");
check("the project page offers the fix in the real window", () => assert.ok(shown));
console.log("screenshot:", await screenshot("real-env-fix-row"));
const rowText = await bodyText();
check("the page says the database cannot be checked while the server is not running", () => {
  assert.ok(
    rowText.includes("Start the project to check whether the database shop exists"),
    rowText,
  );
  assert.ok(!rowText.includes("hunter2"), "the password must not be on the page");
});

assert.ok(await clickButton("Update DB_PORT in .env"), "the button was not found");
const dialogShown = await waitForText("Only this line changes");
check("the dialog shows the line that changes and the backup", () => assert.ok(dialogShown));
const dialogText = await bodyText();
check("with the old and the new line, and no other", () => {
  assert.ok(dialogText.includes("DB_PORT=1111 # old"), dialogText);
  assert.ok(dialogText.includes(`DB_PORT=${String(port)} # old`), dialogText);
  assert.ok(dialogText.includes(".env.lunarun-backup"), dialogText);
  assert.ok(!dialogText.includes("hunter2"), "the password must not be on the page");
});
console.log("screenshot:", await screenshot("real-env-fix-dialog"));
assert.ok(await clickButton("Cancel"), "Cancel was not found");
await new Promise((r) => setTimeout(r, 500));
check("Cancel writes nothing", () => {
  assert.equal(read(), ORIGINAL);
  assert.deepEqual(backups(), []);
});

assert.ok(await clickButton("Update DB_PORT in .env"), "the button was not found again");
await waitForText("Only this line changes");
assert.ok(await clickButton("Update .env"), "Update .env was not found");
const done = await waitForText("DB_PORT updated");
check("Update writes the file and the page says where the backup is", () => assert.ok(done));
check("the backup is the whole original file, byte for byte", () => {
  assert.deepEqual(backups(), [".env.lunarun-backup"]);
  assert.equal(fs.readFileSync(path.join(project, ".env.lunarun-backup"), "utf8"), ORIGINAL);
});
check("only the DB_PORT line changed (CRLF, comment and password untouched)", () => {
  assert.equal(read(), ORIGINAL.replace("DB_PORT=1111 # old", `DB_PORT=${String(port)} # old`));
});
await new Promise((r) => setTimeout(r, 1000));
const after = await invoke("project_env_port_preview", { id });
check("the preview is now empty", () => assert.deepEqual(after, { ok: null }));
console.log("screenshot:", await screenshot("real-env-fix-after"));

// 5. No server running.
const status = await invoke("project_database_status", { id });
check("project_database_status is permitted and says the server is not running", () =>
  assert.deepEqual(status, { ok: "notRunning" }),
);
const create = await invoke("project_database_create", { id });
check("project_database_create is permitted and refuses without a server", () => {
  assert.equal(create.err?.code, "DATABASE_NOT_RUNNING", JSON.stringify(create));
  assert.equal(create.err.details.service, "mysql@8.4");
});
const nope = await invoke("project_env_port_preview", { id: "nope" });
check("an unknown project is NOT_FOUND", () => assert.equal(nope.err?.code, "NOT_FOUND"));

console.log(results.join("\n"));
socket.close();
stop();
