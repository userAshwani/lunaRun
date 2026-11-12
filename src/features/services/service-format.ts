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
