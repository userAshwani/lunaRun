// English is the source language. Other locales must match this shape (see src/i18n/index.ts).
// Keys are grouped by feature. Never concatenate translated fragments: use {{interpolation}}.
export const en = {
  app: {
    name: "lunaRun",
    tagline: "Multi-version PHP & Laravel dev environment.",
    comingSoon: "Coming soon",
  },
} as const;

export type Translation = typeof en;
