import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { onServiceState } from "@/lib/events";
import { ipc } from "@/lib/ipc";

export const SITES_KEY = ["sites", "view"] as const;

/**
 * The site of every project and whether a web server serves them. Caddy starting or stopping changes the second
 * part, so a change of any service asks again: it is cheap, and asking again is always right.
 */
export function useSites() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: SITES_KEY, queryFn: ipc.sitesList });

  useEffect(() => {
    // An object, not a plain boolean: it is set from the cleanup function while the async setup runs.
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;
    void onServiceState(() => {
      void queryClient.invalidateQueries({ queryKey: SITES_KEY });
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

/** "Try again": give Caddy the sites as they are now. The answer is the new view. */
export function useSyncSites() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ipc.sitesSync,
    onSuccess: (view) => {
      queryClient.setQueryData(SITES_KEY, view);
    },
  });
}
