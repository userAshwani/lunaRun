import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { JobInfo } from "@/bindings/JobInfo";
import { onJobDone, onJobProgress } from "@/lib/events";
import { ipc } from "@/lib/ipc";
import { applyProgress } from "./jobs-state";

export const JOBS_KEY = ["jobs"] as const;

/**
 * The background job list, kept live: progress events update the cached list in place (no refetch per
 * event), and a finished or unknown job triggers one refetch so the list is authoritative again.
 */
export function useJobs() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: JOBS_KEY, queryFn: ipc.jobsList });

  useEffect(() => {
    // An object, not a plain boolean: it is set from the cleanup function while the async setup runs.
    const lifecycle = { disposed: false };
    const unsubscribers: (() => void)[] = [];
    const refetch = () => {
      void queryClient.invalidateQueries({ queryKey: JOBS_KEY });
    };

    void (async () => {
      const offProgress = await onJobProgress((progress) => {
        const current = queryClient.getQueryData<JobInfo[]>(JOBS_KEY);
        const next = applyProgress(current, progress);
        if (next) queryClient.setQueryData(JOBS_KEY, next);
        else refetch(); // a job we have not listed yet
      });
      const offDone = await onJobDone(refetch);
      // The component may have unmounted while we were subscribing.
      if (lifecycle.disposed) {
        offProgress();
        offDone();
      } else {
        unsubscribers.push(offProgress, offDone);
      }
    })();

    return () => {
      lifecycle.disposed = true;
      for (const unsubscribe of unsubscribers) unsubscribe();
    };
  }, [queryClient]);

  return query;
}

export function useCancelJob() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (jobId: string) => ipc.jobCancel(jobId),
    // Whatever happened (cancelled, already done, unknown), the list is the truth.
    onSettled: () => queryClient.invalidateQueries({ queryKey: JOBS_KEY }),
  });
}
