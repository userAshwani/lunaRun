import type { JobInfo } from "@/bindings/JobInfo";

export type ServiceAction = "start" | "stop" | "restart";

const KEY_PREFIX = "service:";

/** The service a job is about (`service:mysql@8.4` is `mysql@8.4`), or `null` for a job of another kind. */
export function serviceOfJob(job: Pick<JobInfo, "key">): string | null {
  if (!job.key.startsWith(KEY_PREFIX)) return null;
  const id = job.key.slice(KEY_PREFIX.length);
  return id === "" ? null : id;
}

/**
 * Start, stop or restart? The three share one key (so they can never overlap), and the job carries no other marker
 * than its label, which the backend starts with "Start ", "Stop " or "Restart ".
 */
export function serviceJobAction(job: JobInfo): ServiceAction | null {
  if (serviceOfJob(job) === null) return null;
  if (job.label.startsWith("Restart ")) return "restart";
  if (job.label.startsWith("Start ")) return "start";
  if (job.label.startsWith("Stop ")) return "stop";
  return null;
}

/**
 * The newest job of each service (the list comes newest first, running jobs before finished ones), so the page shows
 * what is happening now or what happened last, never an older attempt.
 */
export function latestServiceJobs(jobs: readonly JobInfo[]): Map<string, JobInfo> {
  const latest = new Map<string, JobInfo>();
  for (const job of jobs) {
    const id = serviceOfJob(job);
    if (id !== null && serviceJobAction(job) !== null && !latest.has(id)) latest.set(id, job);
  }
  return latest;
}
