// English is the source language. Other locales must match this shape (see src/i18n/index.ts).
// Keys are grouped by feature. Never concatenate translated fragments: use {{interpolation}}.
export const en = {
  app: {
    name: "lunaRun",
    tagline: "Multi-version PHP & Laravel dev environment.",
  },
  nav: {
    label: "Main navigation",
    dashboard: "Dashboard",
    projects: "Projects",
    runtimes: "Runtimes",
    services: "Services",
    settings: "Settings",
    diagnostics: "Diagnostics",
  },
  pages: {
    dashboard: { description: "Running services and a quick start for your projects." },
    projects: { description: "Add a project folder and let lunaRun pick the right versions." },
    runtimes: { description: "Install and manage PHP, Node.js, MySQL and more, side by side." },
    services: { description: "Start, stop and inspect web servers, PHP pools and databases." },
    settings: { description: "Defaults, ports, theme and data location." },
    diagnostics: { description: "System information and a report you can copy for bug reports." },
    notImplemented: "This page is not implemented yet.",
  },
  theme: {
    label: "Theme",
    light: "Light",
    dark: "Dark",
    system: "System",
  },
} as const;

export type Translation = typeof en;
