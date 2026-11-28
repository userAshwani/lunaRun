import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { JobInfo } from "@/bindings/JobInfo";
import type { LunaError } from "@/bindings/LunaError";
import type { OpenTarget } from "@/bindings/OpenTarget";
import type { SiteEntry } from "@/bindings/SiteEntry";
import { JOBS_KEY, useCancelJob, useJobs } from "@/features/jobs/use-jobs";
import { IpcError, ipc, toLunaError } from "@/lib/ipc";
import { SITES_KEY } from "./use-sites";

/** The key of a project's start or stop job (`project:<id>`), as the backend names it. */
export const projectJobKey = (projectId: string) => `project:${projectId}`;

export const DATABASE_KEY = ["projects", "database"] as const;

function errorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

export type RunAction = "start" | "stop";

/** Is this job a start or a stop? The label starts with the action, as the backend writes it. */
export function runActionOf(job: JobInfo): RunAction {
  return job.label.startsWith("Stop ") ? "stop" : "start";
}

export interface ProjectRun {
  /** The newest start or stop job of the project, running or finished. */
  job: JobInfo | undefined;
  /** A start or stop is under way. */
  busy: boolean;
  /** Why the last start or stop failed, until it is dismissed or the next attempt. */
  problem: { error: LunaError; action: RunAction } | null;
  start: () => void;
  stop: () => void;
  cancel: () => void;
  dismiss: () => void;
}

/**
 * Starting and stopping one project. It is a background job, so this follows the job list (progress, a failure, Cancel)
 * instead of waiting for the call: the page can be left and come back to the same job.
 */
export function useProjectRun(projectId: string): ProjectRun {
  const queryClient = useQueryClient();
  const jobs = useJobs();
  const cancelJob = useCancelJob();
  const [refused, setRefused] = useState<{ error: LunaError; action: RunAction } | null>(null);
  const [dismissedJob, setDismissedJob] = useState<string | null>(null);
  const job = jobs.data?.find((candidate) => candidate.key === projectJobKey(projectId));

  const mutation = useMutation({
    mutationFn: ({ action }: { action: RunAction }) =>
      action === "start" ? ipc.projectStart(projectId) : ipc.projectStop(projectId),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: JOBS_KEY });
      void queryClient.invalidateQueries({ queryKey: SITES_KEY });
    },
  });
  const run = (action: RunAction) => {
    // A new attempt replaces what the last one left.
    setRefused(null);
    setDismissedJob(job?.id ?? null);
    mutation.mutate(
      { action },
      {
        // A command the backend refuses at once (the quit, another action running).
        onError: (thrown) => {
          setRefused({ error: errorOf(thrown), action });
        },
      },
    );
  };

  const failed = job?.state === "failed" && job.error && job.id !== dismissedJob;
  const problem =
    refused ?? (failed && job.error ? { error: job.error, action: runActionOf(job) } : null);
  return {
    job,
    busy: job?.state === "running" || mutation.isPending,
    problem,
    start: () => {
      run("start");
    },
    stop: () => {
      run("stop");
    },
    cancel: () => {
      if (job?.state === "running") cancelJob.mutate(job.id);
    },
    dismiss: () => {
      setRefused(null);
      setDismissedJob(job?.id ?? null);
    },
  };
}

export type Shown = "running" | "stopped" | "starting" | "stopping" | "notAnswering";

/** What the badge says: the services decide, and a job under way says what it is doing. */
export function shownState(entry: SiteEntry | undefined, run: ProjectRun): Shown {
  if (run.job?.state === "running")
    return runActionOf(run.job) === "stop" ? "stopping" : "starting";
  if (entry?.running) return "running";
  if (entry?.served) return "notAnswering";
  return "stopped";
}

/** Open the site, the folder or the editor of a project. */
export function useOpenProject() {
  return useMutation({
    mutationFn: ({ id, target }: { id: string; target: OpenTarget }) => ipc.projectOpen(id, target),
  });
}

/** How to connect to the database lunaRun runs for a project, when it has one. */
export function useProjectDatabase(projectId: string) {
  return useQuery({
    queryKey: [...DATABASE_KEY, projectId],
    queryFn: () => ipc.projectDatabase(projectId),
  });
}
