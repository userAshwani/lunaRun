import type { AppInfo } from "@/bindings/AppInfo";

/**
 * Plain-text report for bug reports (FR-AP-04). Deliberately not translated, so maintainers can read
 * every report. Contains no secrets: only version, platform and the data folder path.
 */
export function buildReport(info: AppInfo): string {
  return [
    "lunaRun diagnostics",
    `Version: ${info.appVersion}`,
    `OS: ${info.os}`,
    `Architecture: ${info.arch}`,
    `Data folder: ${info.lunaHome}${info.lunaHomeFromEnv ? " (from LUNA_HOME)" : ""}`,
  ].join("\n");
}
