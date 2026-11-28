// The Settings page (D-045) in every state, for settings-shots.mjs and `contrast.mjs --page settings`, against a fake Tauri
// backend (the recipe in TRACKER.md section 7).

const DEFAULTS = {
  defaultPhpLine: null,
  defaultNodeLine: null,
  ports: {
    site: { start: 8100, end: 8999 },
    phpFastcgi: { start: 9100, end: 9999 },
    database: { start: 3306, end: 3399 },
    caddyAdmin: 2119,
  },
  registryUrl: null,
};

const VIEW = {
  current: DEFAULTS,
  defaults: DEFAULTS,
  home: "C:\\Users\\José\\AppData\\Local\\lunaRun",
  homeSource: "default",
  defaultRegistryUrl:
    "https://raw.githubusercontent.com/userAshwani/lunaRun/main/registry/registry.json",
};

const CATALOG = {
  runtimes: [
    {
      id: "php",
      displayName: "PHP",
      license: null,
      versions: ["8.6.0", "8.5.13", "8.4.27", "8.3.36"].map((version) => ({
        runtime: "php",
        version,
        line: version.split(".").slice(0, 2).join("."),
        channel: "stable",
        eol: false,
        eolDate: null,
        securityOnly: false,
        releasedAt: null,
        community: false,
        size: 36000000,
        available: true,
        rosettaFallback: false,
      })),
    },
    {
      id: "node",
      displayName: "Node.js",
      license: null,
      versions: ["26.13.1", "24.12.0", "22.21.0"].map((version) => ({
        runtime: "node",
        version,
        line: version.split(".")[0],
        channel: "stable",
        eol: false,
        eolDate: null,
        securityOnly: false,
        releasedAt: null,
        community: false,
        size: 32000000,
        available: true,
        rosettaFallback: false,
      })),
    },
  ],
  registry: { source: "fresh", ageSeconds: 60, problem: null, refreshing: false },
};

export const SCENARIOS = {
  data: { route: "/settings", view: VIEW, login: false, ready: "Save changes" },
  changed: {
    route: "/settings",
    view: {
      ...VIEW,
      current: {
        ...DEFAULTS,
        defaultPhpLine: "8.4",
        defaultNodeLine: "22",
        registryUrl: "https://mirror.example.org/registry.json",
      },
    },
    login: true,
    ready: "Save changes",
  },
  refused: {
    route: "/settings",
    view: VIEW,
    login: false,
    setError: {
      code: "INVALID_INPUT",
      message: "ports.site and ports.phpFastcgi overlap: a port can only have one use",
      details: { field: "ports.site" },
    },
    steps: [{ fill: ["Sites: From", "9050"] }, { click: "Save changes" }, { wait: "overlap" }],
    ready: "Save changes",
  },
  restart: {
    route: "/settings",
    view: VIEW,
    login: false,
    steps: [
      { fill: ["Web server admin port", "2219"] },
      { click: "Save changes" },
      { wait: "after lunaRun restarts" },
    ],
    ready: "Save changes",
  },
  loading: { route: "/settings", pending: true, ready: "Settings" },
  error: {
    route: "/settings",
    error: { code: "IO", message: "config/settings.json: access is denied" },
    ready: "The settings could not be read",
  },
};

/** Per scenario: [label, text on the page whose colours are measured]. */
export const TARGETS = {
  data: [
    ["save", "Save changes"],
    ["section title", "New projects"],
    ["section text", "do not say which version"],
    ["label", "Default PHP version"],
    ["hint", "It starts in the tray"],
    ["port hint", "Default 8100 to 8999"],
    ["data folder", "AppData"],
    ["badge", "The usual place"],
  ],
  refused: [
    ["save", "Save changes"],
    ["field error", "overlap"],
  ],
  restart: [
    ["save", "Save changes"],
    ["restart note", "after lunaRun restarts"],
    ["restart button", "Restart now"],
  ],
};

/** Script for `page.addInitScript`: the commands the Settings page and the shell call. */
export const bridge = (scenario) => `(() => {
  const s = ${JSON.stringify(scenario)};
  const catalog = ${JSON.stringify(CATALOG)};
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
      if (cmd === "settings_get") {
        if (s.pending) return new Promise(() => {});
        return s.error ? Promise.reject(s.error) : Promise.resolve(s.view);
      }
      if (cmd === "settings_set") return s.setError ? Promise.reject(s.setError) : Promise.resolve({ ...s.view, current: args.settings });
      if (cmd === "launch_at_login_get") return Promise.resolve(s.login ?? false);
      if (cmd === "launch_at_login_set") return Promise.resolve(args.enabled);
      if (cmd === "runtimes_available") return Promise.resolve(catalog);
      if (cmd === "app_info") return Promise.resolve({ appVersion: "0.0.2", os: "windows", arch: "x86_64", lunaHome: "C:\\\\x", lunaHomeFromEnv: false });
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
