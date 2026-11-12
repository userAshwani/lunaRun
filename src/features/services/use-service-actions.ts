import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { JobInfo } from "@/bindings/JobInfo";
import type { LunaError } from "@/bindings/LunaError";
import { useToast } from "@/components/toast-context";
import { JOBS_KEY } from "@/features/jobs/use-jobs";
import { onJobDone } from "@/lib/events";
import { IpcError, ipc, toLunaError } from "@/lib/ipc";
import {
  latestServiceJobs,
  serviceJobAction,
  serviceOfJob,
  type ServiceAction,
} from "./service-jobs";
import { SERVICES_KEY } from "./use-services";

/** Something that went wrong with one start, stop or restart and is still on screen. */
export interface ServiceProblem {
  id: string;
  /** The row the problem belongs to. */
  service: string;
  action: ServiceAction;
  error: LunaError;
}

type Command = ServiceAction | "reassign";

function lunaErrorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

/**
 * Starting, stopping and restarting services, and what is left on screen afterwards. Each command starts a background
 * job on the backend (the page follows it in the jobs list, which `jobs` is). A command the backend refuses at once
 * (the app is quitting, another action is running) is kept as a problem of its own; a job that fails is a problem until
 * it is dismissed or the next attempt replaces it.
 */
export function useServiceActions(jobs: readonly JobInfo[] | undefined) {
  const queryClient = useQueryClient();
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [refused, setRefused] = useState<readonly ServiceProblem[]>([]);
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  const latest = useMemo(() => latestServiceJobs(jobs ?? []), [jobs]);

  const run = useCallback(
    async (command: Command, id: string) => {
      // A new attempt replaces what the last one left.
      setRefused((current) => current.filter((problem) => problem.service !== id));
      setPending((current) => new Set(current).add(id));
      try {
        if (command === "start") await ipc.serviceStart(id);
        else if (command === "stop") await ipc.serviceStop(id);
        else if (command === "restart") await ipc.serviceRestart(id);
        else await ipc.servicePortReassign(id);
      } catch (thrown) {
        const action: ServiceAction = command === "reassign" ? "start" : command;
        setRefused((current) => [
          ...current,
          {
            id: `refused-${id}-${String(Date.now())}`,
            service: id,
            action,
            error: lunaErrorOf(thrown),
          },
        ]);
      } finally {
        setPending((current) => {
          const next = new Set(current);
          next.delete(id);
          return next;
        });
        void queryClient.invalidateQueries({ queryKey: JOBS_KEY });
        void queryClient.invalidateQueries({ queryKey: SERVICES_KEY });
      }
    },
    [queryClient],
  );

  const problems: ServiceProblem[] = [...refused];
  for (const [service, job] of latest) {
    const action = serviceJobAction(job);
    if (job.state !== "failed" || !job.error || !action || dismissed.has(job.id)) continue;
    problems.push({ id: job.id, service, action, error: job.error });
  }

  const dismiss = useCallback((problem: ServiceProblem) => {
    setRefused((current) => current.filter((entry) => entry.id !== problem.id));
    setDismissed((current) => new Set(current).add(problem.id));
  }, []);

  return {
    /** The newest job of each service. */
    latest,
    /** Services whose command has been sent and not answered yet: their buttons wait. */
    pending,
    problems,
    dismiss,
    start: useCallback((id: string) => run("start", id), [run]),
    stop: useCallback((id: string) => run("stop", id), [run]),
    restart: useCallback((id: string) => run("restart", id), [run]),
    useAnotherPort: useCallback((id: string) => run("reassign", id), [run]),
  };
}

/**
 * When a start, stop or restart ends: load the services again, and confirm it with a toast. A failure is not toasted:
 * the page shows it inline, with what to do. `nameOf` turns `mysql@8.4` into "MySQL 8.4".
 */
export function useServiceJobEffects(nameOf: (id: string) => string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const { t } = useTranslation();
  const latest = useRef({ nameOf, toast, t });
  useEffect(() => {
    latest.current = { nameOf, toast, t };
  });

  useEffect(() => {
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;

    void onJobDone((done) => {
      // The job list still has the job as it was while running: its key and label say what it was.
      const job = queryClient.getQueryData<JobInfo[]>(JOBS_KEY)?.find((j) => j.id === done.jobId);
      const id = job ? serviceOfJob(job) : null;
      const action = job ? serviceJobAction(job) : null;
      if (id === null || !action) return;
      void queryClient.invalidateQueries({ queryKey: SERVICES_KEY });
      const { nameOf: name, toast: toaster, t: translate } = latest.current;
      const vars = { name: name(id) };
      if (done.ok) {
        toaster.show({ title: translate(`services.toast.${action}`, vars) });
      } else if (done.error?.code === "CANCELLED") {
        toaster.show({ tone: "info", title: translate("services.toast.cancelled", vars) });
      }
    }).then((off) => {
      if (lifecycle.disposed) off();
      else unsubscribe = off;
    });

    return () => {
      lifecycle.disposed = true;
      unsubscribe?.();
    };
  }, [queryClient]);
}
