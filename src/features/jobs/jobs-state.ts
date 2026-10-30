import type { JobInfo } from "@/bindings/JobInfo";
import type { JobProgress } from "@/bindings/JobProgress";

/**
 * Apply a `job:progress` event to the cached job list. Returns the new list, or `undefined` when the
 * event is about a job we do not know yet (started elsewhere), so the caller should refetch instead.
 */
export function applyProgress(
  jobs: readonly JobInfo[] | undefined,
  progress: JobProgress,
): JobInfo[] | undefined {
  if (!jobs?.some((job) => job.id === progress.jobId)) return undefined;
  return jobs.map((job) =>
    job.id === progress.jobId && job.state === "running"
      ? {
          ...job,
          phase: progress.phase,
          bytesDone: progress.bytesDone,
          bytesTotal: progress.bytesTotal,
          message: progress.message,
        }
      : job,
  );
}

/** 0 to 100, or `null` when the total is unknown (show an indeterminate bar). */
export function percent(bytesDone: number, bytesTotal: number | null): number | null {
  if (bytesTotal === null || bytesTotal <= 0) return null;
  return Math.max(0, Math.min(100, Math.floor((bytesDone / bytesTotal) * 100)));
}
