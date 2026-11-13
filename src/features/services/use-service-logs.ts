import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { LogLine } from "@/bindings/LogLine";
import type { ServiceLogs } from "@/bindings/ServiceLogs";
import { onServiceLog, onServiceState } from "@/lib/events";
import { ipc } from "@/lib/ipc";
import { isMemberOf } from "./service-format";

/** The backend keeps this many lines per source; the page keeps no more either. */
export const MAX_LINES = 2000;

/** One tab of the viewer: one log source of one service. */
export interface LogTab {
  /** Unique per service and source. */
  id: string;
  /** The supervisor's id (`php-fcgi@8.3.36-w2`). */
  service: string;
  /** `output`, or the label of a log file (`MySQL error log`). */
  source: string;
  lines: readonly LogLine[];
}

/** What a snapshot line is called when the same line arrives again as an event. */
function keyOf(service: string, line: LogLine): string {
  return `${service}\u0000${line.source}\u0000${String(line.ts)}\u0000${line.line}`;
}

interface LiveLine {
  service: string;
  line: LogLine;
}

/**
 * The log of one row: what the backend has kept (asked for once, when the panel opens), then every new line as it
 * arrives (`service:log`). The same line can be in both, because a batch may be sent while the snapshot is being
 * made, so lines are told apart by service, source, time and text and shown once. A service that is **started
 * again** begins a new log: the snapshot is asked for again and the lines of the old run are dropped.
 */
export function useServiceLogs(rowId: string) {
  const queryClient = useQueryClient();
  const queryKey = ["services", "logs", rowId] as const;
  const query = useQuery({
    queryKey,
    queryFn: () => ipc.serviceLogs(rowId, MAX_LINES),
    gcTime: 0,
    // A refetch while the user reads would not change what the events already show, but it would flash the loading
    // state for no reason.
    refetchOnWindowFocus: false,
  });
  const [live, setLive] = useState<readonly LiveLine[]>([]);

  useEffect(() => {
    const lifecycle = { disposed: false };
    const unsubscribers: (() => void)[] = [];
    void (async () => {
      const offLog = await onServiceLog((batch) => {
        if (!isMemberOf(rowId, batch.id)) return;
        setLive((current) =>
          [...current, ...batch.lines.map((line) => ({ service: batch.id, line }))].slice(
            -MAX_LINES * 2,
          ),
        );
      });
      const offState = await onServiceState((status) => {
        if (status.state !== "starting" || !isMemberOf(rowId, status.id)) return;
        // A new run: the backend cleared its log, so what is shown from the old run goes too.
        setLive([]);
        void queryClient.invalidateQueries({ queryKey: ["services", "logs", rowId] });
      });
      if (lifecycle.disposed) {
        offLog();
        offState();
      } else {
        unsubscribers.push(offLog, offState);
      }
    })();
    return () => {
      lifecycle.disposed = true;
      for (const off of unsubscribers) off();
    };
  }, [queryClient, rowId]);

  const tabs = useMemo<LogTab[]>(() => {
    const snapshot: readonly ServiceLogs[] = query.data ?? [];
    const services = snapshot.map((entry) => entry.service);
    const order: string[] = [];
    const lines = new Map<string, LogLine[]>();
    const seen = new Set<string>();
    const tabFor = (service: string, source: string) => {
      const id = `${service}\u0000${source}`;
      if (!lines.has(id)) {
        lines.set(id, []);
        order.push(id);
      }
      return lines.get(id) as LogLine[];
    };
    for (const entry of snapshot) {
      for (const source of entry.sources) {
        const target = tabFor(entry.service, source.source);
        for (const line of source.lines) {
          seen.add(keyOf(entry.service, line));
          target.push(line);
        }
      }
    }
    // Lines that arrived as events and are not in the snapshot (a source that did not exist yet makes a tab).
    for (const { service, line } of live) {
      if (!services.includes(service) || seen.has(keyOf(service, line))) continue;
      tabFor(service, line.source).push(line);
    }
    const sortedServices = [...services];
    return order
      .map((id) => {
        const [service = "", source = ""] = id.split("\u0000");
        return { id, service, source, lines: (lines.get(id) ?? []).slice(-MAX_LINES) };
      })
      .sort(
        (a, b) =>
          sortedServices.indexOf(a.service) - sortedServices.indexOf(b.service) ||
          Number(b.source === "output") - Number(a.source === "output") ||
          a.source.localeCompare(b.source),
      );
  }, [query.data, live]);

  return {
    tabs,
    isLoading: query.isPending,
    isError: query.isError,
    error: query.error?.error,
    retry: () => {
      setLive([]);
      void query.refetch();
    },
  };
}
