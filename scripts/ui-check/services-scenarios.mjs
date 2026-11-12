// What the Services page is shown in each state, and the fake Tauri backend that serves it to a plain browser (used by
// services-shots.mjs and contrast.mjs; see README.md). The shapes are the generated `ServiceRow`, `ServiceStatus` and
// `Leftover` types in src/bindings/.

const status = (id, extra = {}) => ({
  id,
  state: "stopped",
  pid: null,
  restarts: 0,
  exitCode: null,
  error: null,
  gaveUp: false,
  killed: false,
  ...extra,
});

const row = (id, extra) => ({
  id,
  line: null,
  ports: [],
  state: "stopped",
  running: 0,
  total: 1,
  members: [status(id)],
  needsSetup: false,
  ...extra,
});

const workers = (pool, count, state) =>
  Array.from({ length: count }, (_, n) =>
    status(`${pool}-w${String(n + 1)}`, { state: state(n), pid: 2000 + n }),
  );

/** One of everything: running, partly up, stopped, never started, crashed, ended by force, stopping. */
export const DATA = [
  row("caddy", {
    kind: "web",
    runtime: "caddy",
    version: "2.11.7",
    state: "running",
    running: 1,
    ports: [2119],
    members: [status("caddy", { state: "running", pid: 1111 })],
  }),
  row("php-fcgi@8.4.12", {
    kind: "php",
    runtime: "php",
    version: "8.4.12",
    state: "running",
    running: 4,
    total: 4,
    ports: [9100, 9101, 9102, 9103],
    members: workers("php-fcgi@8.4.12", 4, () => "running"),
  }),
  row("php-fcgi@8.3.36", {
    kind: "php",
    runtime: "php",
    version: "8.3.36",
    state: "starting",
    running: 3,
    total: 4,
    ports: [9104, 9105, 9106, 9107],
    members: workers("php-fcgi@8.3.36", 4, (n) => (n < 3 ? "running" : "starting")),
  }),
  row("mariadb@12.3", {
    kind: "database",
    runtime: "mariadb",
    version: "12.3.3",
    line: "12.3",
    needsSetup: true,
  }),
  row("mysql@9.7", {
    kind: "database",
    runtime: "mysql",
    version: "9.7.4",
    line: "9.7",
    state: "crashed",
    ports: [3308],
    members: [
      status("mysql@9.7", {
        state: "crashed",
        gaveUp: true,
        restarts: 3,
        error: {
          code: "SERVICE_START_FAILED",
          message: "mysqld.exe exited with code 1 before it was ready",
        },
      }),
    ],
  }),
  row("mysql@8.4", {
    kind: "database",
    runtime: "mysql",
    version: "8.4.13",
    line: "8.4",
    state: "running",
    running: 1,
    ports: [3307],
    members: [status("mysql@8.4", { state: "running", pid: 4444 })],
  }),
  row("mysql@8.0", {
    kind: "database",
    runtime: "mysql",
    version: "8.0.40",
    line: "8.0",
    ports: [3309],
    members: [status("mysql@8.0", { killed: true })],
  }),
  row("mariadb@11.8", {
    kind: "database",
    runtime: "mariadb",
    version: "11.8.9",
    line: "11.8",
    state: "stopping",
    ports: [3310],
    members: [status("mariadb@11.8", { state: "stopping", pid: 5555 })],
  }),
];

export const LEFTOVERS = [
  {
    id: "mysql@8.4",
    pid: 4242,
    exe: "/runtimes/mysql/8.4.13/bin/mysqld",
    startedAt: 1767225600,
    port: 3306,
  },
  {
    id: "caddy",
    pid: 4343,
    exe: "/runtimes/caddy/2.11.7/caddy",
    startedAt: 1767225700,
    port: 2119,
  },
];

/** `ready` is a piece of text that is on the page once the state has settled. */
export const SCENARIOS = {
  data: { list: DATA, leftovers: [], ready: "MySQL 8.4" },
  leftovers: { list: DATA.slice(0, 3), leftovers: LEFTOVERS, ready: "earlier session" },
  empty: { list: [], leftovers: [], ready: "No services yet" },
  error: {
    listError: {
      code: "IO",
      message:
        "C:\\Users\\Lenovo\\AppData\\Local\\lunaRun\\config\\services.json: access is denied (os error 5)",
    },
    leftovers: [],
    ready: "Could not load the services",
  },
  // `services_list` never answers: the skeleton stays.
  loading: { pending: true, leftovers: [], ready: "Loading services" },
};

/**
 * Script for `page.addInitScript`: makes the page believe it runs inside the desktop app (the recipe in TRACKER.md
 * section 7). Commands not part of the screenshot are rejected with a typed error, as the real backend would.
 */
export const bridge = (scenario) => `(() => {
  const s = ${JSON.stringify(scenario)};
  let n = 0;
  window.isTauri = true;
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => ++n,
    unregisterCallback: () => {},
    invoke: (cmd) => {
      if (cmd === "plugin:event|listen") return Promise.resolve(++n);
      if (cmd.startsWith("plugin:event|")) return Promise.resolve();
      if (cmd === "services_list") {
        if (s.pending) return new Promise(() => {});
        return s.listError ? Promise.reject(s.listError) : Promise.resolve(s.list);
      }
      if (cmd === "services_leftovers") return Promise.resolve(s.leftovers);
      if (cmd === "jobs_list") return Promise.resolve([]);
      return Promise.reject({ code: "INTERNAL", message: "not part of this screenshot: " + cmd });
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
})();`;
