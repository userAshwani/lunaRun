// Shared by the Runtimes page tests: sample data and a fake backend for the page.
import { QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { AvailableVersion } from "@/bindings/AvailableVersion";
import type { InstalledView } from "@/bindings/InstalledView";
import type { JobInfo } from "@/bindings/JobInfo";
import type { RegistryStatus } from "@/bindings/RegistryStatus";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import type { RuntimesAvailable } from "@/bindings/RuntimesAvailable";
import { ToastProvider } from "@/components/toast";
import { RuntimesPage } from "@/features/runtimes/runtimes-page";
import { createQueryClient } from "@/lib/query-client";
import { respondTo } from "@/test/backend";

export function version(runtime: string, number: string, extra: Partial<AvailableVersion> = {}) {
  const line = number.split(".").slice(0, 2).join(".");
  return {
    runtime,
    version: number,
    line,
    channel: "stable",
    eol: false,
    eolDate: null,
    securityOnly: false,
    releasedAt: null,
    community: false,
    size: 31_457_280,
    available: true,
    rosettaFallback: false,
    ...extra,
  } satisfies AvailableVersion;
}

export const php: RuntimeCatalog = {
  id: "php",
  displayName: "PHP",
  license: { name: "PHP License v3.01", url: "https://www.php.net/license/" },
  versions: [
    version("php", "8.4.2", { eolDate: "2028-12-31", releasedAt: "2026-10-26" }),
    version("php", "8.3.9", {
      securityOnly: true,
      eolDate: "2027-12-31",
      community: true,
    }),
    version("php", "8.1.30", { eol: true, eolDate: "2025-12-31", size: 1_048_576 }),
    version("php", "7.4.33", { eol: true, eolDate: "2022-11-28" }),
    version("php", "9.0.0", { available: false, size: null }),
  ],
};
export const node: RuntimeCatalog = {
  id: "node",
  displayName: "Node.js",
  license: null,
  versions: [version("node", "24.1.0", { rosettaFallback: true })],
};
export const caddy: RuntimeCatalog = {
  id: "caddy",
  displayName: "Caddy",
  license: null,
  versions: [version("caddy", "2.9.0", { available: false, size: null })],
};

export const fresh: RegistryStatus = {
  source: "fresh",
  ageSeconds: null,
  problem: null,
  refreshing: false,
};

export function available(registry: RegistryStatus = fresh, runtimes = [caddy, node, php]) {
  return { runtimes, registry } satisfies RuntimesAvailable;
}

export const installedPhp: InstalledView = {
  runtime: "php",
  version: "8.3.9",
  dir: "D:\\DIGITALLY BIRD\\lüna home\\runtimes\\php\\8.3.9",
  installedAt: 1_767_225_600, // 2026-01-01 UTC
  rosetta: false,
  parts: 1,
};

export function renderPage() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ToastProvider>
        <RuntimesPage />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

export function backendWith(
  handlers: {
    available?: () => unknown;
    installed?: () => unknown;
    refresh?: () => unknown;
    jobs?: () => unknown;
    /** More commands (`runtime_install`, ...), answered as given. */
    more?: Record<string, (args?: unknown) => unknown>;
  } = {},
) {
  respondTo({
    runtimes_available: handlers.available ?? (() => available()),
    runtimes_installed: handlers.installed ?? (() => [installedPhp]),
    registry_refresh: handlers.refresh ?? (() => fresh),
    jobs_list: handlers.jobs ?? (() => []),
    ...handlers.more,
  });
}

/** A running (or finished) install or uninstall job, as `jobs_list` reports it. */
export function runtimeJob(
  runtime: string,
  version: string,
  extra: Partial<JobInfo> = {},
): JobInfo {
  return {
    id: `job-${runtime}-${version}`,
    key: `${runtime}@${version}`,
    label: `Install ${runtime} ${version}`,
    state: "running",
    phase: "downloading",
    bytesDone: 0,
    bytesTotal: null,
    message: null,
    error: null,
    ...extra,
  };
}
