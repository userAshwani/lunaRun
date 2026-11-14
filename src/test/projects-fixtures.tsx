// Shared by the Projects page tests: sample projects, scans and a fake backend for the pages.
import { QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import type { DetectionReport } from "@/bindings/DetectionReport";
import type { Project } from "@/bindings/Project";
import type { ProjectDetection } from "@/bindings/ProjectDetection";
import type { Recommendation } from "@/bindings/Recommendation";
import type { Resolution } from "@/bindings/Resolution";
import type { Requirement } from "@/bindings/Requirement";
import type { RuntimeResolution } from "@/bindings/RuntimeResolution";
import { ToastProvider } from "@/components/toast";
import { ProjectDetailPage } from "@/features/projects/project-detail-page";
import { ProjectsPage } from "@/features/projects/projects-page";
import { createQueryClient } from "@/lib/query-client";
import { respondTo } from "@/test/backend";
import { available, fresh, installedPhp } from "@/test/runtimes-fixtures";

export function project(id: string, extra: Partial<Project> = {}): Project {
  return {
    id,
    name: id,
    path: `D:\\Work\\${id}`,
    docRoot: "public",
    docRootCustom: false,
    framework: { kind: "laravel", version: "11.9.0", evidence: ["artisan"] },
    bindings: { php: null, node: null },
    lastDetection: { at: 1_767_225_600, hash: "abc" },
    createdAt: 1_767_225_600,
    updatedAt: 1_767_225_600,
    ...extra,
  };
}

export function recommendation(
  version: string,
  extra: Partial<Recommendation> = {},
): Recommendation {
  const line = version.split(".").slice(0, 2).join(".");
  return {
    version,
    line,
    installed: true,
    needsInstall: false,
    eol: false,
    rosettaFallback: false,
    binding: { mode: "line", value: line },
    ...extra,
  };
}

/** PHP resolved to an installed version, from `composer.json`. */
export function readyPhp(
  version = "8.3.14",
  extra: Partial<RuntimeResolution> = {},
): RuntimeResolution {
  return {
    runtime: "php",
    basis: "constraint",
    constraint: ">=8.2.0 <9.0.0",
    recommended: recommendation(version),
    installedMatches: [version],
    issues: [],
    ...extra,
  };
}

/** PHP that has to be installed first. */
export function installPhp(version = "8.3.36"): RuntimeResolution {
  return readyPhp(version, {
    recommended: recommendation(version, { installed: false, needsInstall: true }),
    installedMatches: [],
  });
}

export function requirement(
  constraint = ">=8.2.0 <9.0.0",
  extra: Partial<Requirement> = {},
): Requirement {
  return {
    constraint,
    selector: "constraint",
    pin: null,
    computed: null,
    preferred: null,
    sources: [
      {
        kind: "composerRequire",
        file: "composer.json",
        path: "require.php",
        raw: "^8.2",
        constraint,
      },
      {
        kind: "composerLock",
        file: "composer.lock",
        path: "packages[symfony/console].require.php",
        raw: ">=8.2",
        constraint: ">=8.2.0",
      },
    ],
    ...extra,
  };
}

export function report(extra: Partial<DetectionReport> = {}): DetectionReport {
  return {
    framework: { kind: "laravel", version: "11.9.0", evidence: ["artisan"] },
    docRoot: { path: "public", evidence: "Laravel serves from public/" },
    php: requirement(),
    phpExtensions: [],
    node: null,
    packageManager: null,
    database: null,
    servicesHints: [],
    warnings: [],
    inputsHash: "abc",
    ...extra,
  };
}

export function resolution(extra: Partial<Resolution> = {}): Resolution {
  return { php: readyPhp(), node: null, extensions: [], database: null, ...extra };
}

export function detection(
  p: Project,
  extra: { report?: Partial<DetectionReport>; resolution?: Partial<Resolution> } = {},
): ProjectDetection {
  return {
    project: p,
    report: report(extra.report),
    resolution: resolution(extra.resolution),
  };
}

export const shop = project("shop", { name: "Shop", path: "D:\\Work\\shop" });
export const blog = project("blog", {
  name: "Blog",
  path: "D:\\Work\\blog",
  framework: null,
  docRoot: ".",
});

export function renderList(entry = "/projects") {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <MemoryRouter initialEntries={[entry]}>
        <ToastProvider>
          <Routes>
            <Route path="/projects" element={<ProjectsPage />} />
            <Route path="/projects/:id" element={<ProjectDetailPage />} />
          </Routes>
        </ToastProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

export function backendWith(
  handlers: {
    projects?: () => unknown;
    /** The scan of a project, by id (default: ready PHP). */
    detect?: (id: string) => unknown;
    jobs?: () => unknown;
    more?: Record<string, (args?: unknown) => unknown>;
  } = {},
) {
  const known: Record<string, Project> = { shop, blog };
  const scanOf = handlers.detect ?? ((id: string) => detection(known[id] ?? project(id)));
  respondTo({
    projects_list: handlers.projects ?? (() => [shop, blog]),
    project_detect: (args) => scanOf((args as { id: string }).id),
    runtimes_available: () => available(fresh),
    runtimes_installed: () => [installedPhp],
    jobs_list: handlers.jobs ?? (() => []),
    ...handlers.more,
  });
}
