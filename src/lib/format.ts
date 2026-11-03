const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** `1536` becomes `1.5 KB`. Binary units (1 KB = 1024 B), like the Windows and macOS file dialogs. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // Whole bytes need no decimals, and neither do large round values such as "120 MB".
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit] ?? "B"}`;
}

/** `2027-12-31` (a calendar day from the version list) as a date in the user's language, never shifted by the time zone. */
export function formatIsoDate(iso: string, locale: string): string {
  const day = new Date(`${iso}T00:00:00Z`);
  if (Number.isNaN(day.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeZone: "UTC" }).format(day);
}

/** Seconds since 1970 (as the backend sends them) as a local date in the user's language. */
export function formatUnixDate(seconds: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(new Date(seconds * 1000));
}
