// Shared by the Services page tests: sample rows and a fake backend for the page.
import { QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import type { JobInfo } from "@/bindings/JobInfo";
import type { Leftover } from "@/bindings/Leftover";
import type { ServiceRow } from "@/bindings/ServiceRow";
import type { ServiceStatus } from "@/bindings/ServiceStatus";
import { ToastProvider } from "@/components/toast";
import { ServicesPage } from "@/features/services/services-page";
import { createQueryClient } from "@/lib/query-client";
import { respondTo } from "@/test/backend";

export function status(id: string, extra: Partial<ServiceStatus> = {}): ServiceStatus {
  return {
    id,
    state: "stopped",
    pid: null,
    restarts: 0,
    exitCode: null,
    error: null,
    gaveUp: false,
    killed: false,
    ...extra,
  };
}

/** A row for one service that has not been started: stopped, no ports yet. Override what a test is about. */
export function row(
  id: string,
  extra: Partial<ServiceRow> & Pick<ServiceRow, "kind" | "runtime" | "version">,
): ServiceRow {
  return {
    id,
    line: null,
    ports: [],
    state: "stopped",
    running: 0,
    total: 1,
    members: [status(id)],
    needsSetup: false,
    ...extra,
  };
}

export const caddyRow = row("caddy", { kind: "web", runtime: "caddy", version: "2.11.7" });

export const mysqlRow = row("mysql@8.4", {
  kind: "database",
  runtime: "mysql",
  version: "8.4.13",
  line: "8.4",
  needsSetup: true,
});

/** A Windows PHP pool of four workers, three of them up. */
export const phpPool = row("php-fcgi@8.3.36", {
  kind: "php",
  runtime: "php",
  version: "8.3.36",
  ports: [9100, 9101, 9102, 9103],
  state: "starting",
  running: 3,
  total: 4,
  members: [1, 2, 3, 4].map((n) =>
    status(`php-fcgi@8.3.36-w${String(n)}`, { state: n < 4 ? "running" : "starting" }),
  ),
});

export const phpFpm = row("php-fpm@8.4.12", {
  kind: "php",
  runtime: "php",
  version: "8.4.12",
  ports: [9100],
});

export const leftover: Leftover = {
  id: "mysql@8.4",
  pid: 4242,
  exe: "/runtimes/mysql/8.4.13/bin/mysqld",
  startedAt: 1_767_225_600,
  port: 3306,
};

export function renderPage() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <MemoryRouter>
        <ToastProvider>
          <ServicesPage />
        </ToastProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** A start, stop or restart job as the backend lists it. The label starts with the action, as the backend's does. */
export function serviceJob(
  id: string,
  action: "Start" | "Stop" | "Restart",
  extra: Partial<JobInfo> = {},
): JobInfo {
  return {
    id: `job-${action.toLowerCase()}-${id}`,
    key: `service:${id}`,
    label: `${action} ${id}`,
    state: "running",
    phase: "starting",
    bytesDone: 0,
    bytesTotal: null,
    message: null,
    error: null,
    ...extra,
  };
}

export function backendWith(
  handlers: {
    list?: () => unknown;
    leftovers?: () => unknown;
    jobs?: () => unknown;
    /** Answers for the commands a test is about (`service_start`, `job_cancel`, ...). */
    more?: Record<string, (args?: unknown) => unknown>;
  } = {},
) {
  respondTo({
    services_list: handlers.list ?? (() => [caddyRow, phpPool, mysqlRow]),
    services_leftovers: handlers.leftovers ?? (() => []),
    jobs_list: handlers.jobs ?? (() => []),
    ...handlers.more,
  });
}
