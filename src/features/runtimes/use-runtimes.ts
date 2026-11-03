import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { RuntimesAvailable } from "@/bindings/RuntimesAvailable";
import { onRegistryUpdated } from "@/lib/events";
import { ipc } from "@/lib/ipc";

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
