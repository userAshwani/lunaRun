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

/** A start, stop or restart job as the backend lists it (the label starts with the action, as the real one does). */
const job = (id, action, extra = {}) => ({
  id: `job-${action.toLowerCase()}-${id}`,
  key: `service:${id}`,
  label: `${action} ${id}`,
  state: "running",
  phase: "starting",
  bytesDone: 0,
  bytesTotal: null,
  message: null,
  error: null,
  ...extra,
});

const failed = (id, action, error) => job(id, action, { state: "failed", error });

/** A database's first start, in progress, beside a running Caddy and a stopping service. */
const SETUP = [
  { ...DATA[0] },
  {
    ...DATA[3],
    state: "starting",
    members: [status("mariadb@12.3", { state: "starting", pid: 6666 })],
  },
  { ...DATA[5] },
  { ...DATA[7] },
];

/** Stopped services whose last attempt failed: a taken port (XAMPP), a failed setup, a rejected Caddyfile. */
const PROBLEMS = [
  { ...DATA[0], state: "stopped", running: 0, members: [status("caddy")] },
  { ...DATA[3], needsSetup: true },
  { ...DATA[5], state: "stopped", running: 0, members: [status("mysql@8.4")] },
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

const logLine = (text, source = "output", seconds = 0) => ({
  source,
  line: text,
  ts: Date.UTC(2026, 10, 12, 14, 3, 7) + seconds * 1000,
});

/** Rows for the log scenarios: a running MySQL with two log sources. */
const LOG_ROWS = [DATA[0], DATA[5]];
const LOGS = {
  "mysql@8.4": [
    {
      service: "mysql@8.4",
      sources: [
        {
          source: "output",
          lines: [
            logLine("[lunaRun] starting", "output", 0),
            logLine(
              "mysqld: ready for connections. Version: 8.4.13  socket: ''  port: 3307",
              "output",
              2,
            ),
            logLine("[lunaRun] ready", "output", 3),
          ],
        },
        {
          source: "MySQL error log",
          lines: [
            logLine(
              "2026-11-12T14:03:07.120000Z 0 [System] [MY-010116] [Server] mysqld (mysqld 8.4.13) starting as process 4444",
              "MySQL error log",
              1,
            ),
            logLine(
              "2026-11-12T14:03:09.800000Z 0 [System] [MY-011323] [Server] X Plugin ready for connections",
              "MySQL error log",
              2,
            ),
          ],
        },
      ],
    },
  ],
};
const POOL_LOGS = {
  "php-fcgi@8.3.36": [1, 2, 3, 4].map((n) => ({
    service: `php-fcgi@8.3.36-w${String(n)}`,
    sources: [{ source: "output", lines: [logLine(`[lunaRun] starting`, "output", n)] }],
  })),
};

/** `ready` is a piece of text that is on the page once the state has settled. */
export const SCENARIOS = {
  data: { list: DATA, leftovers: [], ready: "MySQL 8.4" },
  leftovers: { list: [DATA[0], DATA[5], DATA[2]], leftovers: LEFTOVERS, ready: "earlier session" },
  setup: {
    list: SETUP,
    leftovers: [],
    jobs: [
      job("mariadb@12.3", "Start", { phase: "setup" }),
      job("mariadb@11.8", "Stop", { phase: "stopping" }),
    ],
    ready: "Setting up its data folder",
  },
  problems: {
    list: PROBLEMS,
    leftovers: [],
    jobs: [
      failed("mysql@8.4", "Start", {
        code: "PORT_IN_USE",
        message: "port 3307 is already in use by mysqld.exe (C:/xampp/mysql/bin/mysqld.exe)",
        details: {
          port: 3307,
          pid: 4312,
          processName: "mysqld.exe",
          processPath: "C:/xampp/mysql/bin/mysqld.exe",
        },
      }),
      failed("mariadb@12.3", "Start", {
        code: "DATABASE_INIT_FAILED",
        message: "MariaDB 12.3: the setup program failed with exit code 3",
        details: {
          service: "mariadb@12.3",
          log: "C:/Users/Lenovo/lunaRun/logs/mariadb-12.3.init.log",
          tail: ["[ERROR] mariadb-install-db: cannot create directory", "Aborting"],
        },
      }),
      failed("caddy", "Start", {
        code: "CONFIG_INVALID",
        message: "Caddy rejected the generated Caddyfile",
        details: { tail: ["adapting config using caddyfile: unrecognized directive: bind_all"] },
      }),
    ],
    ready: "did not start",
  },
  logs: {
    list: LOG_ROWS,
    leftovers: [],
    logs: LOGS,
    // Steps run after the page is ready: click a button, push an event, wait for some text.
    steps: [
      { click: "Show the logs of MySQL 8.4" },
      { wait: "mysqld: ready for connections" },
      {
        emit: [
          "service:log",
          { id: "mysql@8.4", lines: [logLine("12 new connections accepted", "output", 5)] },
        ],
      },
      { wait: "12 new connections accepted" },
    ],
    ready: "MySQL 8.4",
  },
  "logs-pool": {
    list: [DATA[2]],
    leftovers: [],
    logs: POOL_LOGS,
    steps: [{ click: "Show the logs of PHP 8.3.36" }, { wait: "Worker 1: Output" }],
    ready: "PHP 8.3.36",
  },
  quit: {
    list: LOG_ROWS,
    leftovers: [],
    steps: [
      {
        emit: [
          "shutdown:progress",
          { group: "web", total: 1, done: 1, remaining: [], finished: "caddy", elapsedMs: 300 },
        ],
      },
      {
        emit: [
          "shutdown:progress",
          {
            group: "php",
            total: 4,
            done: 3,
            remaining: ["php-fcgi@8.4.12-w4"],
            finished: "php-fcgi@8.4.12-w3",
            elapsedMs: 900,
          },
        ],
      },
      {
        emit: [
          "shutdown:progress",
          {
            group: "databases",
            total: 2,
            done: 0,
            remaining: ["mysql@8.4", "mariadb@11.8"],
            finished: null,
            elapsedMs: 1500,
          },
        ],
      },
      { wait: "Stopping your services" },
    ],
    ready: "MySQL 8.4",
  },
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
  const listeners = {};
  window.isTauri = true;
  // window.__emit("service:log", payload) delivers an event to everything that listens, as Tauri does.
  window.__emit = (event, payload) => {
    for (const { handler } of listeners[event] ?? []) window["_" + String(handler)]({ event, payload, id: 0 });
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (callback) => {
      n += 1;
      window["_" + String(n)] = callback;
      return n;
    },
    unregisterCallback: () => {},
    invoke: (cmd, args = {}) => {
      if (cmd === "plugin:event|listen") {
        const eventId = ++n;
        (listeners[args.event] ??= []).push({ eventId, handler: args.handler });
        return Promise.resolve(eventId);
      }
      if (cmd.startsWith("plugin:event|")) return Promise.resolve();
      if (cmd === "services_list") {
        if (s.pending) return new Promise(() => {});
        return s.listError ? Promise.reject(s.listError) : Promise.resolve(s.list);
      }
      if (cmd === "services_leftovers") return Promise.resolve(s.leftovers);
      if (cmd === "service_logs") return Promise.resolve(s.logs?.[args.id] ?? []);
      if (cmd === "jobs_list") return Promise.resolve(s.jobs ?? []);
      return Promise.reject({ code: "INTERNAL", message: "not part of this screenshot: " + cmd });
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, eventId) {
      listeners[event] = (listeners[event] ?? []).filter((entry) => entry.eventId !== eventId);
    },
  };
})();`;
