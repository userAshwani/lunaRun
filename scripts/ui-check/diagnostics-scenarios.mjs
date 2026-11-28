// The Diagnostics page (D-046) in every state, for diagnostics-shots.mjs and `contrast.mjs --page diagnostics`, against a
// fake Tauri backend (the recipe in TRACKER.md section 7).

const GB = 1024 * 1024 * 1024;

const REPORT = {
  app: {
    appVersion: "0.0.2",
    os: "windows",
    arch: "x86_64",
    lunaHome: "C:\\Users\\José\\AppData\\Local\\lunaRun",
    lunaHomeFromEnv: false,
  },
  homeSource: "default",
  freeBytes: 40 * GB,
  vcRedist: "ok",
  installed: ["caddy 2.11.9", "mysql 8.4.13", "php 8.6.0"],
  projects: 2,
  ports: [
    { port: 80, assignedTo: null, held: true, byLunarun: false, owner: "httpd.exe" },
    { port: 443, assignedTo: null, held: false, byLunarun: false, owner: null },
    { port: 2119, assignedTo: "caddy", held: true, byLunarun: true, owner: "caddy.exe" },
    { port: 3306, assignedTo: null, held: true, byLunarun: false, owner: "mysqld.exe" },
    { port: 3307, assignedTo: "mysql@8.4", held: true, byLunarun: true, owner: "mysqld.exe" },
    { port: 8080, assignedTo: null, held: false, byLunarun: false, owner: null },
    { port: 8101, assignedTo: "site@4b1e0c", held: false, byLunarun: false, owner: null },
  ],
  services: [
    { id: "mysql@8.4", state: "running", error: null },
    { id: "caddy", state: "error", error: "PORT_IN_USE: port 8101 is used by httpd.exe" },
  ],
  failedJobs: [
    {
      label: "Install Node.js 26.13.1",
      error: "NETWORK: error sending request: connection reset by peer",
    },
  ],
  registry: { source: "cache", ageSeconds: 7200, problem: "NETWORK: offline", customHost: null },
  changedSettings: ["default PHP line = 8.4"],
};

const view = (report) => ({ report, text: "## lunaRun diagnostics\n" });

export const SCENARIOS = {
  data: { route: "/diagnostics", answer: view(REPORT), ready: "Taken by httpd.exe" },
  quiet: {
    route: "/diagnostics",
    answer: view({
      ...REPORT,
      ports: REPORT.ports.map((p) => ({ ...p, held: false, byLunarun: false, owner: null })),
      services: [],
      failedJobs: [],
      registry: { source: "fresh", ageSeconds: 30, problem: null, customHost: null },
      changedSettings: [],
      vcRedist: "missing",
    }),
    ready: "Nothing is running and nothing has gone wrong",
  },
  loading: { route: "/diagnostics", pending: true, ready: "Diagnostics" },
  error: {
    route: "/diagnostics",
    error: {
      code: "PERMISSION_DENIED",
      message: "C:\\Users\\José\\AppData\\Local\\lunaRun: access denied",
    },
    ready: "Could not load system information",
  },
};

/** Per scenario: [label, text on the page whose colours are measured]. */
export const TARGETS = {
  data: [
    ["taken badge", "Taken by httpd.exe"],
    ["note", "leaves out your user name"],
    ["row label", "Free space for the data folder"],
    ["port number", "3307"],
    ["common", "Often used by other programs"],
    ["ours badge", "Used by lunaRun"],
    ["free badge", "Free"],
    ["service error", "PORT_IN_USE"],
    ["failed task", "Install Node.js"],
    ["registry problem", "Last refresh failed"],
    ["check again", "Check again"],
    ["copy", "Copy report"],
  ],
  quiet: [
    ["empty", "Nothing is running and nothing has gone wrong"],
    ["vc missing", "Not installed: PHP for Windows needs it"],
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
      if (cmd === "diagnostics_get") {
        if (s.pending) return new Promise(() => {});
        return s.error ? Promise.reject(s.error) : Promise.resolve(s.answer);
      }
      if (cmd === "app_info") return Promise.resolve(${JSON.stringify(REPORT.app)});
      if (cmd === "jobs_list") return Promise.resolve([]);
      return Promise.reject({ code: "INTERNAL", message: "not part of this screenshot: " + cmd });
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, eventId) {
      listeners[event] = (listeners[event] ?? []).filter((entry) => entry.eventId !== eventId);
    },
  };
})();`;
