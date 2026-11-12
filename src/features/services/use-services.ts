import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { onServiceState } from "@/lib/events";
import { ipc } from "@/lib/ipc";

export const SERVICES_KEY = ["services", "list"] as const;
export const LEFTOVERS_KEY = ["services", "leftovers"] as const;

/**
 * What lunaRun can run, with each service's state. Whenever a service changes state the list is asked for again: it
 * is cheap and read-only, and asking again is always right (a pool's row is a sum of several services, which only the
 * backend adds up).
 */
export function useServices() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: SERVICES_KEY, queryFn: ipc.servicesList });

  useEffect(() => {
    // An object, not a plain boolean: it is set from the cleanup function while the async setup runs.
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;

    void onServiceState(() => {
      void queryClient.invalidateQueries({ queryKey: SERVICES_KEY });
    }).then((off) => {
      if (lifecycle.disposed) off();
      else unsubscribe = off;
    });

    return () => {
      lifecycle.disposed = true;
      unsubscribe?.();
    };
  }, [queryClient]);

  return query;
}

/** Services found still running from an earlier session (found once, when the app started). */
export function useLeftovers() {
  return useQuery({ queryKey: LEFTOVERS_KEY, queryFn: ipc.servicesLeftovers });
}
