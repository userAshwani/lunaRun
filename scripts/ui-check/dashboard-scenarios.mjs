// The Dashboard (D-047) in every state, for dashboard-shots.mjs and `contrast.mjs --page dashboard`, against a fake
// Tauri backend (the recipe in TRACKER.md section 7).

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

const ROWS = [
  {
    id: "caddy",
    kind: "web",
    runtime: "caddy",
    version: "2.11.9",
    line: null,
    ports: [2119, 8101, 8102],
    state: "running",
    running: 1,
    total: 1,
    members: [status("caddy", { state: "running", pid: 4410 })],
    needsSetup: false,
  },
  {
    id: "php-fcgi@8.4.12",
    kind: "php",
    runtime: "php",
    version: "8.4.12",
    line: null,
    ports: [9100, 9101, 9102, 9103],
    state: "running",
    running: 4,
    total: 4,
    members: [1, 2, 3, 4].map((n) => status(`php-fcgi@8.4.12-w${String(n)}`, { state: "running" })),
    needsSetup: false,
  },
  {
    id: "mysql@8.4",
    kind: "database",
    runtime: "mysql",
    version: "8.4.13",
    line: "8.4",
    ports: [3307],
    state: "crashed",
    running: 0,
    total: 1,
    members: [
      status("mysql@8.4", {
        state: "crashed",
        gaveUp: true,
        error: {
          code: "SERVICE_START_FAILED",
          message: "mysqld exited with code 1 (InnoDB: unable to lock ./ibdata1)",
        },
      }),
    ],
    needsSetup: false,
  },
];

const project = (id, name, path, updatedAt) => ({
  id,
  name,
  path,
  docRoot: "public",
  docRootCustom: false,
  framework: { kind: "laravel", version: "12.4.0", evidence: ["artisan"] },
  bindings: { php: null, node: null },
  lastDetection: { at: updatedAt, hash: "abc" },
  createdAt: updatedAt,
  updatedAt,
});

const PROJECTS = [
  project("p1", "Shop", "D:\\Work\\shop", 1_767_225_600),
  project("p2", "Blog", "D:\\Work\\José's blog", 1_767_312_000),
  project("p3", "Admin panel", "D:\\Work\\clients\\acme\\admin-panel", 1_767_100_000),
];

const site = (projectId, port, extra = {}) => ({
  projectId,
  state: "ready",
  name: `${projectId}-4b1e0c`,
  port,
  address: `http://127.0.0.1:${String(port)}`,
  phpVersion: "8.4.12",
  databaseService: null,
  served: false,
  running: false,
  ...extra,
});

const SITES = {
  sites: [
    site("p1", 8101, { served: true, running: true }),
    site("p2", 8102),
    site("p3", null, { state: "noPhp", address: null, phpVersion: null }),
  ],
  webServer: "running",
  problem: null,
};

const INSTALLED = [
  ["caddy", "2.11.9"],
  ["php", "8.4.12"],
  ["php", "8.3.36"],
  ["node", "24.12.0"],
  ["mysql", "8.4.13"],
].map(([runtime, version]) => ({
  runtime,
  version,
  dir: "",
  installedAt: 1_767_000_000,
  rosetta: false,
  parts: 1,
}));

const JOBS = [
  {
    id: "j1",
    key: "node@26.13.1",
    label: "Install Node.js 26.13.1",
    state: "failed",
    phase: "download",
    bytesDone: 0,
    bytesTotal: null,
    message: null,
    error: { code: "NETWORK", message: "error sending request: connection reset by peer" },
  },
];

const EMPTY = {
  services: [],
  projects: [],
  sites: { sites: [], webServer: "notInstalled", problem: null },
  installed: [],
  jobs: [],
};

export const SCENARIOS = {
  busy: {
    route: "/",
    answers: { services: ROWS, projects: PROJECTS, sites: SITES, installed: INSTALLED, jobs: JOBS },
    ready: "MySQL 8.4 crashed and was not restarted again",
  },
  healthy: {
    route: "/",
    answers: {
      services: ROWS.slice(0, 2),
      projects: PROJECTS,
      sites: SITES,
      installed: INSTALLED,
      jobs: [],
    },
    ready: "No problems",
  },
  fresh: { route: "/", answers: EMPTY, ready: "Get started" },
  started: {
    route: "/",
    answers: {
      ...EMPTY,
      installed: INSTALLED.slice(0, 2),
      services: ROWS.slice(0, 1).map((r) => ({
        ...r,
        state: "stopped",
        running: 0,
        ports: [],
        members: [status("caddy")],
      })),
    },
    ready: "Get started",
  },
  loading: { route: "/", pending: true, ready: "Dashboard" },
  error: {
    route: "/",
    answers: { ...EMPTY, installed: INSTALLED },
    fail: {
      code: "STATE_CORRUPT",
      message:
        "C:\\Users\\José\\AppData\\Local\\lunaRun\\config\\projects.json: expected value at line 1 column 1",
    },
    ready: "The projects could not be read",
  },
};

/** Per scenario: [label, text on the page whose colours are measured]. */
export const TARGETS = {
  busy: [
    ["problem title", "MySQL 8.4 crashed and was not restarted again"],
    ["problem message", "mysqld exited with code 1"],
    ["failed task", "Install Node.js 26.13.1 failed"],
    ["project name", "Blog"],
    ["project address", "http://127.0.0.1:8101"],
    ["project path", "D:\\Work\\José's blog"],
    ["running badge", "Running"],
    ["stopped badge", "Stopped"],
    ["service ports", "Port 9100–9103"],
    ["stat hint", "of 3 services"],
    ["stat runtimes", "Caddy, PHP, Node.js, MySQL"],
    ["section description", "What lunaRun runs for your projects right now."],
    ["view link", "Open Diagnostics"],
  ],
  healthy: [
    ["no problems", "No problems"],
    ["no problems text", "Every service is healthy and no task has failed."],
  ],
  fresh: [
    ["start title", "Install your runtimes"],
    ["empty projects", "No projects yet"],
    ["empty services", "Nothing is running"],
    ["stat zero hint", "Nothing installed yet"],
  ],
  started: [["done step", "Install your runtimes"]],
  error: [
    ["error title", "The projects could not be read"],
    ["error message", "expected value at line 1"],
    ["unavailable", "Could not be read"],
  ],
};

/** Script for `page.addInitScript`: the commands the page and the shell call. */
export const bridge = (scenario) => `(() => {
  const s = ${JSON.stringify(scenario)};
  let n = 0;
  const listeners = {};
  window.isTauri = true;
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
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
      if (cmd === "onboarding_get") return Promise.resolve({ show: false });
      if (cmd === "app_info") return Promise.resolve({ appVersion: "0.0.2", os: "windows", arch: "x86_64", lunaHome: "", lunaHomeFromEnv: false });
      const key = { services_list: "services", projects_list: "projects", sites_list: "sites", runtimes_installed: "installed", jobs_list: "jobs" }[cmd];
      if (key) {
        if (s.pending) return new Promise(() => {});
        if (s.fail && key === "projects") return Promise.reject(s.fail);
        return Promise.resolve(s.answers[key]);
      }
      return Promise.reject({ code: "INTERNAL", message: "not part of this screenshot: " + cmd });
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, eventId) {
      listeners[event] = (listeners[event] ?? []).filter((entry) => entry.eventId !== eventId);
    },
  };
})();`;
