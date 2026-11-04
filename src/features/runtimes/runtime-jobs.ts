import type { JobInfo } from "@/bindings/JobInfo";

export type JobKind = "install" | "uninstall";

/** The job key the backend uses for both installing and uninstalling a version. */
export function jobKey(runtime: string, version: string): string {
  return `${runtime}@${version}`;
}

/** `php@8.3.27` as its two parts, or `null` for a job that is not about one runtime version. */
export function parseJobKey(key: string): { runtime: string; version: string } | null {
  const at = key.indexOf("@");
  if (at <= 0 || at === key.length - 1) return null;
  return { runtime: key.slice(0, at), version: key.slice(at + 1) };
}

/**
 * Install or uninstall? Both use the same key (so they can never overlap), and the job carries no other
 * marker than its label, which the backend starts with "Install " or "Uninstall ". Anything else is not a
 * runtime job (`null`).
 */
export function jobKind(job: JobInfo): JobKind | null {
  if (parseJobKey(job.key) === null) return null;
  if (job.label.startsWith("Uninstall ")) return "uninstall";
  if (job.label.startsWith("Install ")) return "install";
  return null;
}

/**
 * The newest job for each runtime version (the list comes newest first, running jobs before finished ones),
 * so the page shows what is happening now or what happened last, never an older attempt.
 */
export function latestRuntimeJobs(jobs: readonly JobInfo[]): Map<string, JobInfo> {
  const latest = new Map<string, JobInfo>();
  for (const job of jobs) {
    if (jobKind(job) !== null && !latest.has(job.key)) latest.set(job.key, job);
  }
  return latest;
}
