import { Boxes, FolderKanban, Server } from "lucide-react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/page-header";
import { StatCard } from "@/components/stat-card";
import { useJobs } from "@/features/jobs/use-jobs";
import { runtimeName } from "@/features/projects/project-format";
import { useProjects, useRescanWhenRuntimesChange } from "@/features/projects/use-projects";
import { useSites } from "@/features/projects/use-sites";
import { useInstalledRuntimes } from "@/features/runtimes/use-runtimes";
import { useServices } from "@/features/services/use-services";
import { activeServices, currentProblems, quickProjects } from "./dashboard-model";
import { GetStarted } from "./get-started";
import { ProblemsCard } from "./problems-card";
import { QuickProjects } from "./quick-projects";
import { RunningServices } from "./running-services";

/**
 * The page a developer opens every day (D-047): what runs, the projects with Start and Stop, and what is wrong. It reads
 * what the other pages read and follows the same events (`service:state`, the jobs); it never polls.
 */
export function DashboardPage() {
  const { t } = useTranslation();
  const services = useServices();
  const projects = useProjects();
  const sites = useSites();
  const installed = useInstalledRuntimes();
  const jobs = useJobs();
  // Start enables when an install gives a project its PHP: the sites are read again when a job ends.
  useRescanWhenRuntimesChange();

  const rows = services.data ?? [];
  const active = activeServices(rows);
  const running = rows.filter((row) => row.state === "running").length;
  const projectList = projects.data ?? [];
  const shown = quickProjects(projectList, sites.data?.sites);
  const servedCount = sites.data?.sites.filter((site) => site.served).length ?? 0;
  const problems = currentProblems(
    rows,
    jobs.data ?? [],
    sites.data?.problem ?? null,
    new Set(shown.map((project) => project.id)),
  );
  const installedCount = installed.data?.length ?? 0;
  const runtimeNames = [
    ...new Set((installed.data ?? []).map((item) => runtimeName(item.runtime))),
  ];

  // Only once both are known: a flash of "Get started" while the lists load would be untrue.
  const setUp = installedCount > 0 && projectList.length > 0;
  const showStart = installed.isSuccess && projects.isSuccess && !setUp;

  let servicesHint: string;
  if (services.isError) servicesHint = t("dashboard.stats.unavailable");
  else if (rows.length === 0) servicesHint = t("dashboard.stats.services.none");
  else servicesHint = t("dashboard.stats.services.of", { count: rows.length });

  let projectsHint: string;
  if (projects.isError) projectsHint = t("dashboard.stats.unavailable");
  else if (projectList.length === 0) projectsHint = t("dashboard.stats.projects.hint");
  else if (servedCount === 0) projectsHint = t("dashboard.stats.projects.noneRunning");
  else projectsHint = t("dashboard.stats.projects.running", { count: servedCount });

  let runtimesHint: string;
  if (installed.isError) runtimesHint = t("dashboard.stats.unavailable");
  else if (runtimeNames.length === 0) runtimesHint = t("dashboard.stats.runtimes.hint");
  else runtimesHint = runtimeNames.join(", ");

  return (
    <>
      <PageHeader title={t("nav.dashboard")} description={t("pages.dashboard.description")} />

      <div className="grid grid-cols-[repeat(auto-fit,minmax(11rem,1fr))] gap-4">
        <StatCard
          icon={Server}
          label={t("dashboard.stats.services.label")}
          value={services.isError ? "–" : running}
          hint={servicesHint}
          loading={services.isPending}
        />
        <StatCard
          icon={FolderKanban}
          label={t("dashboard.stats.projects.label")}
          value={projects.isError ? "–" : projectList.length}
          hint={projectsHint}
          loading={projects.isPending}
        />
        <StatCard
          icon={Boxes}
          label={t("dashboard.stats.runtimes.label")}
          value={installed.isError ? "–" : installedCount}
          hint={runtimesHint}
          loading={installed.isPending}
        />
      </div>

      {showStart && (
        <div className="mt-6">
          <GetStarted
            done={{
              runtimes: installedCount > 0,
              projects: projectList.length > 0,
              services: servedCount > 0,
            }}
          />
        </div>
      )}

      <div className="mt-6 grid items-start gap-6 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <QuickProjects
          shown={shown}
          total={projectList.length}
          sites={sites.data?.sites}
          sitesLoading={sites.isPending}
          loading={projects.isPending}
          error={projects.error?.error ?? null}
          onRetry={() => void projects.refetch()}
        />
        <div className="grid min-w-0 gap-6">
          <RunningServices
            active={active}
            loading={services.isPending}
            error={services.error?.error ?? null}
            onRetry={() => void services.refetch()}
          />
          <ProblemsCard
            problems={problems}
            loading={services.isPending || jobs.isPending}
            error={services.error?.error ?? jobs.error?.error ?? null}
            onRetry={() => {
              void services.refetch();
              void jobs.refetch();
            }}
          />
        </div>
      </div>
    </>
  );
}
