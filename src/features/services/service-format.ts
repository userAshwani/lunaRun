import type { ServiceRow } from "@/bindings/ServiceRow";

/** Product names are not translated. */
const RUNTIME_NAMES: Readonly<Record<string, string>> = {
  php: "PHP",
  mysql: "MySQL",
  mariadb: "MariaDB",
  caddy: "Caddy",
};

/** `MySQL 8.4`, `PHP 8.3.36`, `Caddy`: what the row is called. A database belongs to its line, PHP to its exact version. */
export function serviceName(row: Pick<ServiceRow, "runtime" | "version" | "line">): string {
  const name = RUNTIME_NAMES[row.runtime] ?? row.runtime;
  if (row.line) return `${name} ${row.line}`;
  if (row.runtime === "php") return `${name} ${row.version}`;
  return name;
}

/**
 * Ports as short text: `3307`, `9100–9103` for a run of consecutive ports (a Windows PHP pool), `8100, 8105`
 * otherwise. Empty for no ports.
 */
export function formatPorts(ports: readonly number[]): string {
  const sorted = [...new Set(ports)].sort((a, b) => a - b);
  // Runs of consecutive ports, as [first, last].
  const runs: [number, number][] = [];
  for (const port of sorted) {
    const run = runs.at(-1);
    if (run && port === run[1] + 1) run[1] = port;
    else runs.push([port, port]);
  }
  return runs
    .map(([first, last]) => (last > first ? `${String(first)}–${String(last)}` : String(first)))
    .join(", ");
}

/** Is `service` (a supervisor id) one of the services of the row `rowId`? A Windows pool's workers are `<pool>-w<n>`. */
export function isMemberOf(rowId: string, service: string): boolean {
  if (service === rowId) return true;
  const prefix = `${rowId}-w`;
  return service.startsWith(prefix) && /^\d+$/.test(service.slice(prefix.length));
}

/** The number of a pool worker (`php-fcgi@8.3.36-w2` is 2), or `null` for any other service. */
export function workerNumber(service: string): number | null {
  const found = /-w(\d+)$/.exec(service);
  return found?.[1] ? Number(found[1]) : null;
}

/** `14:03:07` in the user's own clock and format: when lunaRun read a log line. */
export function formatLogTime(milliseconds: number): string {
  return new Date(milliseconds).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}
