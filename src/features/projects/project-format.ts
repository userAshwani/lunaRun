import type { DetectionReport } from "@/bindings/DetectionReport";
import type { RuntimeResolution } from "@/bindings/RuntimeResolution";

/** What a runtime column or a checklist row says about one runtime of a project. */
export type RuntimeState =
  | { kind: "ready"; version: string }
  | { kind: "install"; version: string }
  | { kind: "none" }
  | { kind: "notRequired" };

/** Ready (an installed version fits), to install (one that could be), none (nothing fits), or not required. */
export function runtimeState(resolution: RuntimeResolution | null | undefined): RuntimeState {
  if (!resolution) return { kind: "notRequired" };
  const chosen = resolution.recommended;
  if (!chosen) return { kind: "none" };
  return chosen.needsInstall
    ? { kind: "install", version: chosen.version }
    : { kind: "ready", version: chosen.version };
}

/** The scan found no folder: it was moved or deleted after it was added. */
export function folderIsMissing(report: DetectionReport | undefined): boolean {
  return report?.warnings.some((warning) => warning.code === "PROJECT_FOLDER_MISSING") ?? false;
}

/** The name of a runtime as people write it. */
export function runtimeName(runtime: string): string {
  switch (runtime) {
    case "php":
      return "PHP";
    case "node":
      return "Node.js";
    case "mysql":
      return "MySQL";
    case "mariadb":
      return "MariaDB";
    default:
      return runtime;
  }
}
