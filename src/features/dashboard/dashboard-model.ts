// What the Dashboard shows, worked out from what the other pages already read (D-047). Pure: no React, no IPC.
import type { JobInfo } from "@/bindings/JobInfo";
import type { LunaError } from "@/bindings/LunaError";
import type { Project } from "@/bindings/Project";
import type { ServiceRow } from "@/bindings/ServiceRow";
import type { SiteEntry } from "@/bindings/SiteEntry";
import { isMemberOf } from "@/features/services/service-format";

/** How many projects the Dashboard lists; the rest are one click away on the Projects page. */
export const QUICK_PROJECTS = 5;

/**
 * The projects to list, the ones being served first (they are what the user is working on now), then the most
 * recently added or changed. Starting a project does not change `updatedAt`: a start is not an edit, and writing
 * `projects.json` on every start would be a write for nothing (D-047).
 */
export function quickProjects(
  projects: readonly Project[],
  sites: readonly SiteEntry[] | undefined,
): Project[] {
  const served = new Set(sites?.filter((site) => site.served).map((site) => site.projectId));
  return [...projects]
    .sort(
      (a, b) =>
        Number(served.has(b.id)) - Number(served.has(a.id)) ||
        b.updatedAt - a.updatedAt ||
        a.name.localeCompare(b.name),
    )
    .slice(0, QUICK_PROJECTS);
}

/** The services doing something: running, or on their way up or down. A service in trouble is a problem instead. */
export function activeServices(rows: readonly ServiceRow[]): ServiceRow[] {
  return rows.filter(
    (row) => row.state === "running" || row.state === "starting" || row.state === "stopping",
  );
}

export type Problem =
  /** A service crashed (`gaveUp`: its automatic restarts ran out) or stopped with an error. */
  | { kind: "service"; row: ServiceRow; crashed: boolean; gaveUp: boolean; error: LunaError | null }
  /** A background task whose last attempt failed: an install, an uninstall, a start or a stop. */
  | { kind: "job"; job: JobInfo; projectId: string | null }
  /** Caddy refused the last change of the sites (D-038). */
  | { kind: "sites"; error: LunaError };

/**
 * What is wrong **now**. A failed task only counts while it is the last attempt for its key: an install that failed
 * and then worked is not a problem any more. The job list is running jobs first, then finished ones newest first, so
 * the first job of a key is its current outcome. A service's failed start is told once, by the service; a project's
 * failed start is told by its row when the project is listed (`listedProjects`).
 */
export function currentProblems(
  rows: readonly ServiceRow[],
  jobs: readonly JobInfo[],
  sitesProblem: LunaError | null,
  listedProjects: ReadonlySet<string>,
): Problem[] {
  const problems: Problem[] = [];
  for (const row of rows) {
    const troubled = row.members.filter((m) => m.state === "crashed" || m.state === "error");
    if (troubled.length === 0) continue;
    problems.push({
      kind: "service",
      row,
      crashed: troubled.some((m) => m.state === "crashed"),
      gaveUp: troubled.some((m) => m.gaveUp),
      error: troubled.find((m) => m.error)?.error ?? null,
    });
  }

  const told = problems.flatMap((p) => (p.kind === "service" ? [p.row.id] : []));
  const seen = new Set<string>();
  for (const job of jobs) {
    if (seen.has(job.key)) continue;
    seen.add(job.key);
    if (job.state !== "failed") continue;
    if (job.key.startsWith("service:")) {
      const service = job.key.slice("service:".length);
      if (told.some((rowId) => isMemberOf(rowId, service))) continue;
    }
    const projectId = job.key.startsWith("project:") ? job.key.slice("project:".length) : null;
    if (projectId && listedProjects.has(projectId)) continue;
    problems.push({ kind: "job", job, projectId });
  }

  if (sitesProblem) problems.push({ kind: "sites", error: sitesProblem });
  return problems;
}

/** Where a failed task is fixed: the project's page, the Services page, or the Runtimes page (installs). */
export function jobPage(job: JobInfo): string {
  if (job.key.startsWith("project:")) return `/projects/${job.key.slice("project:".length)}`;
  if (job.key.startsWith("service:")) return "/services";
  return "/runtimes";
}
