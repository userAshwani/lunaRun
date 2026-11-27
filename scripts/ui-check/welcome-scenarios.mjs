// The first-run wizard (D-044) in every state, for welcome-shots.mjs and `contrast.mjs --page welcome`, against a fake
// Tauri backend (the recipe in TRACKER.md section 7).

const MB = 1024 * 1024;
const GB = 1024 * MB;

const VIEW = {
  show: true,
  home: "C:\\Users\\José\\AppData\\Local\\lunaRun",
  homeSource: "default",
  canChooseHome: true,
  checks: { vcRedist: "ok", freeBytes: 40 * GB, enoughSpace: true },
  pack: {
    items: [
      { runtime: "php", version: "8.6.0", line: "8.6", size: 36 * MB, installedVersion: null },
      {
        runtime: "composer",
        version: "2.11.3",
        line: "2.11",
        size: 3 * MB,
        installedVersion: null,
      },
      { runtime: "node", version: "24.12.0", line: "24", size: 32 * MB, installedVersion: null },
      { runtime: "mysql", version: "8.4.13", line: "8.4", size: 290 * MB, installedVersion: null },
    ],
    unavailable: [],
  },
};

const job = (runtime, version, state, extra = {}) => ({
  id: `job-${runtime}`,
  key: `${runtime}@${version}`,
  label: `Install ${runtime} ${version}`,
  state,
  phase: "downloading",
  bytesDone: 18 * MB,
  bytesTotal: 36 * MB,
  message: null,
  error: null,
  ...extra,
});

export const SCENARIOS = {
  home: { route: "/welcome", view: VIEW, ready: "Choose another folder" },
  "home-confirm": {
    route: "/welcome",
    view: VIEW,
    pick: "D:\\OneDrive\\lunaRun",
    checked: { path: "D:\\OneDrive\\lunaRun", warnings: ["synced"] },
    steps: [{ click: "Choose another folder" }, { wait: "Use this folder and restart" }],
    ready: "Choose another folder",
  },
  "home-refused": {
    route: "/welcome",
    view: VIEW,
    pick: "D:\\Work",
    checkError: {
      code: "INVALID_INPUT",
      message: "D:\\Work already has other files in it: choose an empty folder or a new one",
      details: { field: "home", reason: "notEmpty" },
    },
    steps: [{ click: "Choose another folder" }, { wait: "That folder cannot be used" }],
    ready: "Choose another folder",
  },
  "home-env": {
    route: "/welcome",
    view: { ...VIEW, homeSource: "env", canChooseHome: false },
    ready: "Set by the LUNA_HOME environment variable",
  },
  checks: {
    route: "/welcome",
    view: VIEW,
    steps: [{ click: "Continue" }, { wait: "free where the data folder is" }],
    ready: "Choose another folder",
  },
  "checks-problems": {
    route: "/welcome",
    view: { ...VIEW, checks: { vcRedist: "missing", freeBytes: GB / 2, enoughSpace: false } },
    steps: [{ click: "Continue" }, { wait: "Only 512" }],
    ready: "Choose another folder",
  },
  pack: {
    route: "/welcome",
    view: {
      ...VIEW,
      pack: {
        items: [
          ...VIEW.pack.items.slice(0, 3),
          { ...VIEW.pack.items[3], installedVersion: "8.4.12" },
        ],
        unavailable: [],
      },
    },
    steps: [{ click: "Continue" }, { click: "Continue" }, { wait: "Downloads about" }],
    ready: "Choose another folder",
  },
  "pack-installing": {
    route: "/welcome",
    view: VIEW,
    jobs: [
      job("php", "8.6.0", "running"),
      job("composer", "2.11.3", "succeeded"),
      job("node", "24.12.0", "failed", {
        error: { code: "NETWORK", message: "error sending request: connection reset by peer" },
      }),
    ],
    steps: [
      { click: "Continue" },
      { click: "Continue" },
      { click: "Install 4 runtimes" },
      { wait: "Go to the dashboard" },
    ],
    ready: "Choose another folder",
  },
  loading: { route: "/welcome", pending: true, ready: "Welcome to lunaRun" },
  error: {
    route: "/welcome",
    error: { code: "IO", message: "config/onboarding.json: access is denied" },
    ready: "The setup could not be prepared",
  },
};

/** Per scenario: [label, text on the page whose colours are measured] (`contrast.mjs --page welcome`). */
export const TARGETS = {
  home: [
    ["title", "Welcome to lunaRun"],
    ["description", "Three short steps"],
    ["step: current", "Data folder"],
    ["step: other", "This computer"],
    ["folder", "AppData"],
    ["source badge", "The usual place"],
    ["skip", "Skip setup"],
  ],
  "home-confirm": [
    ["skip", "Skip setup"],
    ["confirm text", "lunaRun will restart"],
    ["warning", "synchronised by a cloud service"],
  ],
  "home-refused": [
    ["skip", "Skip setup"],
    ["problem title", "That folder cannot be used"],
    ["problem text", "It already has other files"],
  ],
  "checks-problems": [
    ["skip", "Skip setup"],
    ["missing", "is not installed"],
    ["why", "PHP for Windows needs it"],
    ["low space", "Only 512"],
  ],
  pack: [
    ["skip", "Skip setup"],
    ["item", "PHP 8.6.0"],
    ["size", "36.0 MB"],
    ["installed badge", "Installed (8.4.12)"],
    ["total", "Downloads about"],
  ],
  "pack-installing": [
    ["skip", "Skip setup"],
    ["phase", "Downloading"],
    ["done badge", "Installed"],
    ["failed badge", "Could not install"],
    ["later", "keep running in the background"],
  ],
};

/** Script for `page.addInitScript`: the commands the wizard calls, answered from the scenario. */
export const bridge = (scenario) => `(() => {
  const s = ${JSON.stringify(scenario)};
  let n = 0;
  let started = false;
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
      if (cmd === "plugin:dialog|open") return Promise.resolve(s.pick ?? null);
      if (cmd === "onboarding_get") {
        if (s.pending) return new Promise(() => {});
        return s.error ? Promise.reject(s.error) : Promise.resolve(s.view);
      }
      if (cmd === "onboarding_check_home") return s.checkError ? Promise.reject(s.checkError) : Promise.resolve(s.checked);
      if (cmd === "runtime_install") {
        started = true;
        return Promise.resolve({ id: "job-" + args.runtime, alreadyRunning: false });
      }
      if (cmd === "jobs_list") return Promise.resolve(started ? (s.jobs ?? []) : []);
      return Promise.reject({ code: "INTERNAL", message: "not part of this screenshot: " + cmd });
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, eventId) {
      listeners[event] = (listeners[event] ?? []).filter((entry) => entry.eventId !== eventId);
    },
  };
})();`;
