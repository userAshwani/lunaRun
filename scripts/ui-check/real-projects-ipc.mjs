// Calls the project commands through the REAL Tauri bridge of the real app window, with no browser library: the release
// app is started with WebView2's remote debugging port open, and a minimal Chrome DevTools Protocol client (Node's own
// WebSocket) evaluates `window.__TAURI_INTERNALS__.invoke(...)` in the page. That is what proves a new command is
// permitted by `capabilities/default.json` (unit tests mock the backend and cannot), and that the errors reach the page
// as `{ code, message, details }`. Windows only (WebView2); the folder names have a space and a non-ASCII letter (NFR-07).
//
//   pnpm tauri build --no-bundle
//   node scripts/ui-check/real-projects-ipc.mjs <path to lunarun.exe> <scratch folder>
//
// It makes `<scratch>/lüna home` (the app's data folder) and `<scratch>/mon projet é` (a project), and ends the app.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const [exe, scratch] = process.argv.slice(2);
if (!exe || !scratch) {
  console.error(
    "usage: node scripts/ui-check/real-projects-ipc.mjs <lunarun.exe> <scratch folder>",
  );
  process.exit(2);
}
const PORT = 9333;
const home = path.join(path.resolve(scratch), "lüna home");
const project = path.join(path.resolve(scratch), "mon projet é");
fs.rmSync(home, { recursive: true, force: true });
fs.rmSync(project, { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });
fs.mkdirSync(path.join(project, "public"), { recursive: true });
fs.writeFileSync(path.join(project, "artisan"), "");
fs.writeFileSync(path.join(project, "public", "index.php"), "<?php\n");
fs.writeFileSync(
  path.join(project, "composer.json"),
  JSON.stringify({ require: { php: "^8.2", "laravel/framework": "^11.0" } }),
);
fs.writeFileSync(
  path.join(project, ".env"),
  "DB_CONNECTION=mysql\nDB_PORT=3306\nDB_PASSWORD=never-in-a-report\n",
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
const evaluate = (expression) =>
  new Promise((resolve) => {
    const id = nextId++;
    waiting.set(id, resolve);
    socket.send(
      JSON.stringify({
        id,
        method: "Runtime.evaluate",
        params: { expression, awaitPromise: true, returnByValue: true },
      }),
    );
  });

/** `{ ok: value }` or `{ err: LunaError }`, as the page would see it. */
async function invoke(command, args = {}) {
  const reply = await evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})
       .then((value) => ({ ok: value }), (err) => ({ err }))`,
  );
  assert.ok(reply.result?.result, `no answer from ${command}: ${JSON.stringify(reply)}`);
  return reply.result.result.value;
}

// The window has to be up and the app's setup done before the first command works.
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

const empty = await invoke("projects_list");
check("projects_list starts empty and is permitted", () => assert.deepEqual(empty, { ok: [] }));

const added = await invoke("project_add", { path: project });
check("project_add reads the folder", () => {
  assert.ok(added.ok, JSON.stringify(added));
  const { project: p, report, resolution } = added.ok;
  assert.equal(p.name, "mon projet é");
  assert.equal(p.docRoot, "public");
  assert.equal(p.framework.kind, "laravel");
  assert.equal(report.docRoot.path, "public");
  assert.equal(report.database.engine, "mysql");
  assert.ok(resolution.php.basis);
  assert.ok(
    !JSON.stringify(added.ok).includes("never-in-a-report"),
    "the password must not reach the page",
  );
});
const id = added.ok?.project?.id;

const again = await invoke("project_add", { path: project });
check("project_add twice is PROJECT_EXISTS with the id", () => {
  assert.equal(again.err?.code, "PROJECT_EXISTS");
  assert.equal(again.err.details.id, id);
});
const relative = await invoke("project_add", { path: "relative/folder" });
check("project_add of a relative path is INVALID_PATH", () =>
  assert.equal(relative.err?.code, "INVALID_PATH"),
);

const got = await invoke("project_get", { id });
check("project_get", () => assert.equal(got.ok?.id, id));
const listed = await invoke("projects_list");
check("projects_list has it", () =>
  assert.deepEqual(
    listed.ok?.map((p) => p.id),
    [id],
  ),
);

const updated = await invoke("project_update", {
  id,
  patch: { name: "Shop é", php: { kind: "use", binding: { mode: "line", value: "8.3" } } },
});
check("project_update renames and binds", () => {
  assert.equal(updated.ok?.name, "Shop é");
  assert.deepEqual(updated.ok.bindings.php, { mode: "line", value: "8.3" });
});
const bad = await invoke("project_update", {
  id,
  patch: { docRoot: { kind: "use", path: "../outside" } },
});
check("a document root outside the project is INVALID_INPUT", () => {
  assert.equal(bad.err?.code, "INVALID_INPUT");
  assert.equal(bad.err.details.field, "docRoot");
});
const detected = await invoke("project_detect", { id });
check("project_detect re-scans and the hand-made binding decides", () => {
  assert.equal(detected.ok?.resolution.php.basis, "manual");
});
const gone = await invoke("project_get", { id: "nope" });
check("an unknown id is NOT_FOUND", () => assert.equal(gone.err?.code, "NOT_FOUND"));

const removed = await invoke("project_remove", { id });
check("project_remove", () => assert.deepEqual(removed, { ok: null }));
check("and leaves the folder alone", () => {
  assert.ok(fs.existsSync(path.join(project, "artisan")));
  assert.ok(fs.existsSync(path.join(project, ".env")));
});
const saved = JSON.parse(fs.readFileSync(path.join(home, "config", "projects.json"), "utf8"));
check("projects.json is on disk with a schema and no project left", () => {
  assert.equal(saved.schema, 1);
  assert.deepEqual(saved.projects, []);
});

console.log(results.join("\n"));
socket.close();
stop();
