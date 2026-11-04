import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { JobInfo } from "@/bindings/JobInfo";
import type { RuntimesAvailable } from "@/bindings/RuntimesAvailable";
import { useToast } from "@/components/toast-context";
import { JOBS_KEY } from "@/features/jobs/use-jobs";
import { onJobDone, onRegistryUpdated } from "@/lib/events";
import { ipc } from "@/lib/ipc";
import { jobKind, parseJobKey } from "./runtime-jobs";

export const AVAILABLE_KEY = ["runtimes", "available"] as const;
export const INSTALLED_KEY = ["runtimes", "installed"] as const;

/**
 * Every runtime with its installable versions, plus where the list came from. The app refreshes the list in
 * the background after it starts, so this listens for that to finish and loads the new list.
 */
export function useRuntimesAvailable() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: AVAILABLE_KEY, queryFn: ipc.runtimesAvailable });

  useEffect(() => {
    // An object, not a plain boolean: it is set from the cleanup function while the async setup runs.
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;
    const reload = () => {
      void queryClient.invalidateQueries({ queryKey: AVAILABLE_KEY });
    };

    void onRegistryUpdated(reload).then((off) => {
      if (lifecycle.disposed) {
        off();
        return;
      }
      unsubscribe = off;
      // The refresh may have finished between the first load and the subscription. Only then is a reload
      // needed: reloading unconditionally would also quietly retry (and hide) a failed first load.
      const loaded = queryClient.getQueryData<RuntimesAvailable>(AVAILABLE_KEY);
      if (loaded?.registry.refreshing) reload();
    });

    return () => {
      lifecycle.disposed = true;
      unsubscribe?.();
    };
  }, [queryClient]);

  return query;
}

export function useInstalledRuntimes() {
  return useQuery({ queryKey: INSTALLED_KEY, queryFn: ipc.runtimesInstalled });
}

/** "Check for updates": look for a fresher version list now. */
export function useRegistryRefresh() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ipc.registryRefresh,
    // The list may have changed, and the status line must show the outcome either way.
    onSettled: () => queryClient.invalidateQueries({ queryKey: AVAILABLE_KEY }),
  });
}

/** Start an install. The job it starts shows up in the jobs list, which drives the progress on the row. */
export function useInstallRuntime() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ runtime, version }: { runtime: string; version: string }) =>
      ipc.runtimeInstall(runtime, version),
    onSettled: () => queryClient.invalidateQueries({ queryKey: JOBS_KEY }),
  });
}

export interface UninstallRequest {
  runtime: string;
  version: string;
  deleteData: boolean;
  confirmBoundProjects: boolean;
}

export function useUninstallRuntime() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ runtime, version, ...options }: UninstallRequest) =>
      ipc.runtimeUninstall(runtime, version, options),
    onSettled: () => queryClient.invalidateQueries({ queryKey: JOBS_KEY }),
  });
}

/** What uses a version and where its databases are. Asked fresh each time the dialog opens. */
export function useRuntimeUsage(runtime: string, version: string) {
  return useQuery({
    queryKey: ["runtimes", "usage", runtime, version],
    queryFn: () => ipc.runtimeUsage(runtime, version),
    gcTime: 0,
  });
}

/**
 * When an install or uninstall ends: reload what is installed, and confirm it with a toast. A failure is not
 * toasted: the page shows it inline, with what to do. `nameOf` turns `php` into "PHP".
 */
export function useRuntimeJobEffects(nameOf: (runtime: string) => string) {
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
      void queryClient.invalidateQueries({ queryKey: INSTALLED_KEY });
      // The job list still has the job as it was while running: its key and label say what it was.
      const job = queryClient.getQueryData<JobInfo[]>(JOBS_KEY)?.find((j) => j.id === done.jobId);
      const parts = job ? parseJobKey(job.key) : null;
      const kind = job ? jobKind(job) : null;
      if (!parts || !kind) return;
      const { nameOf: name, toast: toaster, t: translate } = latest.current;
      const vars = { name: name(parts.runtime), version: parts.version };
      if (done.ok) {
        toaster.show({
          title: translate(
            kind === "install" ? "runtimes.toast.installed" : "runtimes.toast.uninstalled",
            vars,
          ),
        });
      } else if (done.error?.code === "CANCELLED") {
        toaster.show({
          tone: "info",
          title: translate(
            kind === "install"
              ? "runtimes.toast.installCancelled"
              : "runtimes.toast.uninstallCancelled",
            vars,
          ),
        });
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
