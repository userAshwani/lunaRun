// What the Projects pages are shown in each state, and the fake Tauri backend that serves them to a plain browser (used by
// projects-shots.mjs and contrast.mjs --page projects; see README.md). The shapes are the generated `Project`,
// `ProjectDetection` and related types in src/bindings/.

const project = (id, extra = {}) => ({
  id,
  name: id,
  path: `D:\\Work\\${id}`,
  docRoot: "public",
  docRootCustom: false,
  framework: { kind: "laravel", version: "11.9.0", evidence: ["artisan"] },
  bindings: { php: null, node: null },
  lastDetection: { at: 1_767_225_600, hash: "abc" },
  createdAt: 1_767_225_600,
  updatedAt: 1_767_225_600,
  ...extra,
});

const recommendation = (version, extra = {}) => ({
  version,
  line: version.split(".").slice(0, 2).join("."),
  installed: true,
  needsInstall: false,
  eol: false,
  rosettaFallback: false,
  binding: { mode: "line", value: version.split(".").slice(0, 2).join(".") },
  ...extra,
});

const runtime = (name, version, extra = {}) => ({
  runtime: name,
  basis: "constraint",
  constraint: ">=8.2.0 <9.0.0",
  recommended: recommendation(version),
  installedMatches: [version],
  issues: [],
  ...extra,
});

const requirement = (constraint = ">=8.2.0 <9.0.0", extra = {}) => ({
  constraint,
  selector: "constraint",
  pin: null,
  computed: null,
  preferred: null,
  sources: [
    {
      kind: "composerRequire",
      file: "composer.json",
      path: "require.php",
      raw: "^8.2",
      constraint,
    },
    {
      kind: "composerLock",
      file: "composer.lock",
      path: "packages[symfony/console].require.php",
      raw: ">=8.2",
      constraint: ">=8.2.0",
    },
  ],
  ...extra,
});

const report = (extra = {}) => ({
  framework: { kind: "laravel", version: "11.9.0", evidence: ["artisan"] },
  docRoot: { path: "public", evidence: "Laravel serves from public/" },
  php: requirement(),
  phpExtensions: [],
  node: null,
  packageManager: null,
  database: null,
  servicesHints: [],
  warnings: [],
  inputsHash: "abc",
  ...extra,
});

const detection = (p, parts = {}) => ({
  project: p,
  report: report(parts.report),
  resolution: {
    php: runtime("php", "8.3.14"),
    node: null,
    extensions: [],
    database: null,
    ...parts.resolution,
  },
});

const SHOP = project("shop", { name: "Shop", path: "D:\\Work\\shop" });
const BLOG = project("blog", {
  name: "Blog",
  path: "D:\\Work\\blog",
  framework: null,
  docRoot: ".",
});
const LEGACY = project("legacy", { name: "Legacy site", path: "D:\\Work\\old sites\\legacy-café" });
const GONE = project("gone", { name: "Moved away", path: "D:\\Work\\moved-away" });

const NODE_REQ = requirement("lts/*", {
  selector: "latestLts",
  sources: [{ kind: "nvmRc", file: ".nvmrc", path: "", raw: "lts/*", constraint: "lts/*" }],
});

const FULL = detection(SHOP, {
  report: {
    node: NODE_REQ,
    database: {
      engine: "mysql",
      connection: "mysql",
      host: "127.0.0.1",
      port: 3307,
      database: "shop",
      username: "root",
      file: ".env",
    },
    packageManager: "pnpm@9.1.0",
    warnings: [
      {
        code: "PIN_CONFLICTS_WITH_COMPOSER",
        file: ".php-version",
        message: "english",
        details: ["8.1", ">=8.2.0 <9.0.0"],
      },
      { code: "DEV_BRANCH", file: "composer.json", message: "english", details: ["dev-main"] },
    ],
  },
  resolution: {
    node: runtime("node", "22.11.0", { constraint: "lts/*" }),
    extensions: [
      {
        name: "intl",
        iniName: "intl",
        neededBy: ["filament/filament", "Laravel"],
        availability: "absent",
      },
      { name: "mbstring", iniName: "mbstring", neededBy: ["Laravel"], availability: "enabled" },
    ],
    database: {
      engine: "mysql",
      runtime: "mysql",
      line: "8.4",
      installedVersion: "8.4.3",
      note: "none",
    },
  },
});

const NEEDS_INSTALL = detection(BLOG, {
  report: {
    framework: null,
    docRoot: { path: ".", evidence: "index.php is in the project folder" },
  },
  resolution: {
    php: runtime("php", "8.3.36", {
      recommended: recommendation("8.3.36", { installed: false, needsInstall: true }),
      installedMatches: [],
    }),
  },
});

const NO_FIT = detection(LEGACY, {
  report: { php: requirement(">=7.4.0 <8.0.0") },
  resolution: {
    php: runtime("php", "7.4.33", {
      constraint: ">=7.4.0 <8.0.0",
      recommended: null,
      installedMatches: [],
      issues: [{ code: "NO_COMPATIBLE_VERSION", message: "english", unavailableLines: ["7.4"] }],
    }),
  },
});

const MISSING = detection(GONE, {
  report: {
    php: null,
    warnings: [{ code: "PROJECT_FOLDER_MISSING", file: null, message: "english", details: [] }],
  },
});

const PHP_CATALOG = {
  id: "php",
  displayName: "PHP",
  license: null,
  versions: ["8.4.2", "8.3.36", "8.2.28", "8.1.30"].map((version) => ({
    runtime: "php",
    version,
    line: version.split(".").slice(0, 2).join("."),
    channel: "stable",
    eol: version.startsWith("8.1"),
    eolDate: null,
    securityOnly: false,
    releasedAt: null,
    community: false,
    size: 31_457_280,
    available: true,
    rosettaFallback: false,
  })),
};

const SCANS = { shop: FULL, blog: NEEDS_INSTALL, legacy: NO_FIT, gone: MISSING };

const LIST = [SHOP, BLOG, LEGACY, GONE];

const INSTALL_JOB = {
  id: "job-1",
  key: "php@8.3.36",
  label: "Install php 8.3.36",
  state: "running",
  phase: "downloading",
  bytesDone: 12_000_000,
  bytesTotal: 31_457_280,
  message: null,
  error: null,
};

const FAILED_JOB = {
  ...INSTALL_JOB,
  state: "failed",
  phase: "downloading",
  error: { code: "NETWORK", message: "error sending request: connection reset by peer" },
};

/** Every scenario names its `route`, the `scans` it answers, and the text that says the page is ready. */
export const SCENARIOS = {
  list: { route: "/projects", projects: LIST, scans: SCANS, ready: "Legacy site" },
  "list-empty": { route: "/projects", projects: [], scans: {}, ready: "No projects yet" },
  "list-error": {
    route: "/projects",
    listError: {
      code: "IO",
      message: "C:\\Users\\Ada\\AppData\\Local\\lunaRun\\config\\projects.json: access is denied",
    },
    ready: "Could not load your projects",
  },
  "list-loading": { route: "/projects", pending: true, ready: "Add a project folder" },
  "list-add-error": {
    route: "/projects",
    projects: LIST,
    scans: SCANS,
    pick: "D:\\Work\\notes.txt",
    addError: { code: "INVALID_PATH", message: "D:\\Work\\notes.txt is not a folder that exists" },
    steps: [{ click: "Add project" }, { wait: "Could not add that folder" }],
    ready: "Legacy site",
  },
  "list-add-exists": {
    route: "/projects",
    projects: LIST,
    scans: SCANS,
    pick: "D:\\Work\\shop",
    addError: {
      code: "PROJECT_EXISTS",
      message: "D:\\Work\\shop is already the project Shop",
      details: { id: "shop", name: "Shop" },
    },
    steps: [{ click: "Add project" }, { wait: "is already in your projects" }],
    ready: "Legacy site",
  },
  "list-drop": {
    route: "/projects",
    projects: LIST,
    scans: SCANS,
    steps: [
      { emit: ["tauri://drag-enter", { paths: ["D:\\Work\\new"], position: { x: 10, y: 10 } }] },
      { wait: "Drop a folder" },
    ],
    ready: "Legacy site",
  },
  detail: { route: "/projects/shop", projects: LIST, scans: SCANS, ready: "Things to look at" },
  "detail-install": {
    route: "/projects/blog",
    projects: LIST,
    scans: SCANS,
    ready: "needed and not installed",
  },
  "detail-installing": {
    route: "/projects/blog",
    projects: LIST,
    scans: SCANS,
    jobs: [INSTALL_JOB],
    ready: "Downloading",
  },
  "detail-install-failed": {
    route: "/projects/blog",
    projects: LIST,
    scans: SCANS,
    jobs: [FAILED_JOB],
    ready: "connection reset",
  },
  "detail-nofit": {
    route: "/projects/legacy",
    projects: LIST,
    scans: SCANS,
    ready: "No PHP version fits",
  },
  "detail-missing": {
    route: "/projects/gone",
    projects: LIST,
    scans: SCANS,
    ready: "The project folder is missing",
  },
  "detail-scan-error": {
    route: "/projects/shop",
    projects: LIST,
    scans: {},
    scanError: { code: "IO", message: "composer.lock: the file is locked by another program" },
    ready: "Could not read this project",
  },
  "detail-loading": {
    route: "/projects/shop",
    projects: LIST,
    scans: {},
    scanPending: true,
    ready: "What this project needs",
  },
  "detail-notfound": {
    route: "/projects/nothing",
    projects: LIST,
    scans: SCANS,
    ready: "not in your list",
  },
  "detail-remove": {
    route: "/projects/shop",
    projects: LIST,
    scans: SCANS,
    steps: [{ click: "Remove Shop" }, { wait: "are not touched" }],
    ready: "Things to look at",
  },
};

/** Texts whose contrast is measured (contrast.mjs --page projects), per scenario. The first one is the "ready" text. */
export const TARGETS = {
  list: [
    ["page description", "Add a project folder and let"],
    ["table header", "Node.js"],
    ["project name", "Shop"],
    ["project path", "D:\\Work\\shop"],
    ["framework", "Laravel 11.9.0"],
    ["badge: ready", "PHP 8.3.14"],
    ["badge: install", "PHP 8.3.36"],
    ["badge: no match", "No match"],
    ["badge: folder missing", "Folder missing"],
    ["not required", "Not required"],
  ],
  "list-empty": [
    ["empty title", "No projects yet"],
    ["empty description", "Add a PHP or Laravel project folder"],
  ],
  "list-error": [
    ["error title", "Could not load your projects"],
    ["error detail", "access is denied"],
  ],
  "list-add-error": [
    ["row name (the page is ready)", "Legacy site"],
    ["problem title", "Could not add that folder"],
    ["problem category", "Invalid path"],
    ["problem message", "is not a folder that exists"],
  ],
  "list-add-exists": [
    ["row name (the page is ready)", "Legacy site"],
    ["problem text", "is already in your projects"],
    ["problem link", "Open Shop"],
  ],
  "list-drop": [
    ["row name (the page is ready)", "Legacy site"],
    ["drop title", "Drop a folder to add it"],
    ["drop hint", "lunaRun only reads the folder"],
  ],
  detail: [
    ["checklist title", "What this project needs"],
    ["php title", "PHP 8.3.14 is installed"],
    ["php basis", "From the project files."],
    ["sources link", "Where this comes from"],
    ["extensions title", "PHP extensions: 1 required extension"],
    ["extension line", "Needed by filament"],
    ["extensions hint", "Enabling extensions comes in a later version"],
    ["node title", "Node.js 22.11.0 is installed"],
    ["database title", "Database: MySQL 8.4 is installed"],
    ["database port", "The project expects it on port 3307."],
    ["warning text", "pins a PHP version that composer.json does not allow"],
    ["warning detail", "8.1 · >=8.2.0 <9.0.0"],
    ["about label", "Package manager"],
    ["about value", "pnpm@9.1.0"],
    ["last scanned", "Last scanned"],
    ["settings label", "Document root"],
    ["settings hint", "The folder that is served"],
    ["settings binding hint", "Automatic picks the best installed version"],
  ],
  "detail-install": [
    ["php title", "PHP 8.3.36 is needed and not installed yet"],
    ["install button", "Install PHP 8.3.36"],
  ],
  "detail-installing": [
    ["progress phase", "Downloading"],
    ["progress bytes", "of"],
    ["cancel button", "Cancel"],
  ],
  "detail-install-failed": [
    ["problem message", "connection reset"],
    ["retry button", "Try again"],
  ],
  "detail-nofit": [
    ["problem title", "No PHP version fits this project"],
    ["issue text", "would fit, but there is no download of it"],
  ],
  "detail-missing": [["warning", "The project folder is missing"]],
  "detail-scan-error": [
    ["error title", "Could not read this project"],
    ["error detail", "locked by another program"],
  ],
  "detail-notfound": [
    ["empty title", "This project is not in your list"],
    ["empty description", "It may have been removed"],
  ],
  "detail-remove": [
    ["checklist (the page is ready)", "Things to look at"],
    ["dialog title", "Remove Shop?"],
    ["dialog text", "are not touched"],
  ],
};

/**
 * Script for `page.addInitScript`: makes the page believe it runs inside the desktop app (the recipe in TRACKER.md
 * section 7), with the commands the Projects pages call. Anything else is rejected with a typed error, as the real
 * backend would.
 */
export const bridge = (scenario) => `(() => {
  const s = ${JSON.stringify(scenario)};
  const catalog = ${JSON.stringify(PHP_CATALOG)};
  let n = 0;
  const listeners = {};
  window.isTauri = true;
  window.__emit = (event, payload) => {
    for (const { handler } of listeners[event] ?? []) window["_" + String(handler)]({ event, payload, id: 0 });
  };
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
      if (cmd === "projects_list") {
        if (s.pending) return new Promise(() => {});
        return s.listError ? Promise.reject(s.listError) : Promise.resolve(s.projects);
      }
      if (cmd === "project_detect") {
        if (s.scanPending) return new Promise(() => {});
        if (s.scanError) return Promise.reject(s.scanError);
        const found = s.scans?.[args.id];
        return found ? Promise.resolve(found) : Promise.reject({ code: "NOT_FOUND", message: "no project " + args.id });
      }
      if (cmd === "project_add") return Promise.reject(s.addError ?? { code: "INTERNAL", message: "not part of this screenshot" });
      if (cmd === "runtimes_available") {
        return Promise.resolve({
          runtimes: [catalog],
          registry: { source: "fresh", ageSeconds: null, problem: null, refreshing: false },
        });
      }
      if (cmd === "runtimes_installed") {
        return Promise.resolve([
          { runtime: "php", version: "8.3.14", dir: "x", installedAt: 1767225600, rosetta: false, parts: 1 },
        ]);
      }
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
