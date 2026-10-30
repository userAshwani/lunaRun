// English is the source language. Other locales must match this shape (see src/i18n/index.ts).
// Keys are grouped by feature. Never concatenate translated fragments: use {{interpolation}}.
export const en = {
  app: {
    name: "lunaRun",
    tagline: "PHP dev environment",
    version: "Version {{version}}",
  },
  nav: {
    label: "Main navigation",
    dashboard: "Dashboard",
    projects: "Projects",
    runtimes: "Runtimes",
    services: "Services",
    settings: "Settings",
    diagnostics: "Diagnostics",
    groups: {
      workspace: "Workspace",
      environment: "Environment",
      system: "System",
    },
  },
  pages: {
    dashboard: { description: "An overview of your local development environment." },
    projects: { description: "Add a project folder and let lunaRun pick the right versions." },
    runtimes: { description: "Install and manage PHP, Node.js, MySQL and more, side by side." },
    services: { description: "Start, stop and inspect web servers, PHP pools and databases." },
    settings: { description: "Defaults, ports, theme and data location." },
    diagnostics: { description: "System information and a report you can copy for bug reports." },
    notImplemented: {
      title: "Coming soon",
      description: "This section is still being built and will appear in an upcoming release.",
    },
  },
  dashboard: {
    stats: {
      services: { label: "Running services", hint: "No services are running" },
      projects: { label: "Projects", hint: "No projects added yet" },
      runtimes: { label: "Installed runtimes", hint: "Nothing installed yet" },
    },
    start: {
      title: "Get started",
      description: "Three steps from a fresh install to a Laravel site in your browser.",
      runtimes: {
        title: "Install your runtimes",
        description:
          "Pick the PHP, Node.js and database versions you need. Keep as many as you like.",
        action: "Open Runtimes",
      },
      projects: {
        title: "Add a project",
        description:
          "Choose a folder. lunaRun reads composer.json and recommends matching versions.",
        action: "Open Projects",
      },
      services: {
        title: "Start your services",
        description: "Run the web server, PHP and database for a project with one click.",
        action: "Open Services",
      },
    },
  },
  common: {
    retry: "Try again",
  },
  // One entry per ErrorCode (src/bindings/ErrorCode.ts). Type-checked: a missing code fails the build.
  errors: {
    NOT_FOUND: "Not found",
    PERMISSION_DENIED: "Permission denied",
    IO: "File system error",
    PATH_OUTSIDE_ROOT: "Path not allowed",
    INVALID_PATH: "Invalid path",
    STATE_CORRUPT: "A data file was damaged",
    STATE_SCHEMA_TOO_NEW: "Data was created by a newer lunaRun",
    NETWORK: "Network problem",
    CHECKSUM_MISMATCH: "Download failed verification",
    CANCELLED: "Cancelled",
    INVALID_URL: "Invalid or insecure address",
    INTERNAL: "Unexpected error",
  },
  diagnostics: {
    loading: "Loading system information",
    loadError: "Could not load system information",
    system: {
      title: "System",
      description:
        "Details about this installation. The data folder path may contain your user name.",
    },
    fields: {
      version: "lunaRun version",
      os: "Operating system",
      arch: "Processor architecture",
      home: "Data folder",
    },
    os: {
      windows: "Windows",
      macos: "macOS",
      linux: "Linux",
      unknown: "Unknown",
    },
    arch: {
      x86_64: "x86-64 (Intel / AMD)",
      aarch64: "ARM64 (Apple silicon / ARM)",
      unknown: "Unknown",
    },
    home: {
      fromEnv: "Set by LUNA_HOME",
      default: "Default location",
    },
    copy: {
      action: "Copy report",
      done: "Copied",
      failed: "Copy failed",
    },
  },
  theme: {
    label: "Theme",
    light: "Light",
    dark: "Dark",
    system: "System",
  },
} as const;

export type Translation = typeof en;
