import { ExternalLink, FolderKanban, LoaderCircle, Play, Square, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { LunaError } from "@/bindings/LunaError";
import type { Project } from "@/bindings/Project";
import type { SiteEntry } from "@/bindings/SiteEntry";
import { EmptyState } from "@/components/empty-state";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { RunStateBadge } from "@/features/projects/run-card";
import {
  runActionOf,
  shownState,
  useOpenProject,
  useProjectRun,
} from "@/features/projects/use-project-run";
import { useErrorCategory } from "@/lib/errors";
import { IpcError, toLunaError } from "@/lib/ipc";
import { QUICK_PROJECTS } from "./dashboard-model";
import { Section, SectionError, SectionSkeleton } from "./section";

interface QuickProjectsProps {
  /** The projects to list, in order (`quickProjects`). */
  shown: readonly Project[];
  /** How many projects there are in all. */
  total: number;
  sites: readonly SiteEntry[] | undefined;
  sitesLoading: boolean;
  loading: boolean;
  error: LunaError | null;
  onRetry: () => void;
}

/** The projects with Start or Stop and Open in browser: the same actions as the project page and the tray (D-039, D-043). */
export function QuickProjects({
  shown,
  total,
  sites,
  sitesLoading,
  loading,
  error,
  onRetry,
}: QuickProjectsProps) {
  const { t } = useTranslation();
  let body;
  if (loading) {
    body = <SectionSkeleton label={t("dashboard.projects.loading")} rows={3} />;
  } else if (error) {
    body = <SectionError error={error} title={t("dashboard.projects.failed")} onRetry={onRetry} />;
  } else if (shown.length === 0) {
    body = (
      <EmptyState
        compact
        icon={FolderKanban}
        title={t("dashboard.projects.emptyTitle")}
        description={t("dashboard.projects.emptyDescription")}
        action={
          <Button asChild size="sm">
            <Link to="/projects">{t("dashboard.projects.add")}</Link>
          </Button>
        }
      />
    );
  } else {
    body = (
      <ul className="divide-y px-6 pb-2">
        {shown.map((project) => (
          <ProjectRow
            key={project.id}
            project={project}
            entry={sites?.find((site) => site.projectId === project.id)}
            sitesLoading={sitesLoading}
          />
        ))}
      </ul>
    );
  }
  return (
    <Section
      title={t("dashboard.projects.title")}
      description={t("dashboard.projects.description")}
      action={
        total > 0 && (
          <Button asChild variant="ghost" size="sm" className="text-primary-soft-foreground">
            <Link to="/projects">
              {total > QUICK_PROJECTS
                ? t("dashboard.projects.viewAll", { count: total })
                : t("dashboard.projects.open")}
            </Link>
          </Button>
        )
      }
    >
      {body}
    </Section>
  );
}

function ProjectRow({
  project,
  entry,
  sitesLoading,
}: {
  project: Project;
  entry: SiteEntry | undefined;
  sitesLoading: boolean;
}) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const run = useProjectRun(project.id);
  const open = useOpenProject();
  const [openProblem, setOpenProblem] = useState<LunaError | null>(null);
  const state = shownState(entry, run);
  const canStart = entry?.state === "ready";
  const stopping = run.job?.state === "running" && runActionOf(run.job) === "stop";
  const starting = run.job?.state === "running" && !stopping;

  let problem: string | null = null;
  if (run.problem) {
    const reason = categoryOf(run.problem.error.code);
    problem =
      run.problem.action === "start"
        ? t("dashboard.projects.startFailed", { reason })
        : t("dashboard.projects.stopFailed", { reason });
  } else if (openProblem) {
    problem = categoryOf(openProblem.code);
  }

  return (
    <li className="flex items-center gap-3 py-3">
      <div className="min-w-0 flex-1">
        <Link
          to={`/projects/${project.id}`}
          className="block truncate font-medium hover:underline focus-visible:underline"
        >
          {project.name}
        </Link>
        <p className="truncate text-[13px] text-muted-foreground tabular-nums">
          {entry?.served && entry.address ? entry.address : project.path}
        </p>
        {problem && (
          <p role="alert" className="mt-0.5 text-[13px] text-destructive">
            {problem}{" "}
            <Link
              to={`/projects/${project.id}`}
              className="font-medium underline underline-offset-2"
            >
              {t("dashboard.projects.details")}
            </Link>
          </p>
        )}
      </div>
      {sitesLoading && !entry ? <Skeleton className="h-6 w-20" /> : <RunStateBadge state={state} />}
      <div className="flex shrink-0 items-center gap-1">
        <Button
          variant="ghost"
          size="icon"
          className="size-8"
          disabled={!entry?.running || !entry.address || open.isPending}
          aria-label={t("dashboard.projects.openLabel", { name: project.name })}
          title={t("projects.run.actions.browser")}
          onClick={() => {
            setOpenProblem(null);
            open.mutate(
              { id: project.id, target: "browser" },
              {
                onError: (thrown) => {
                  setOpenProblem(thrown instanceof IpcError ? thrown.error : toLunaError(thrown));
                },
              },
            );
          }}
        >
          <ExternalLink aria-hidden />
        </Button>
        {starting ? (
          <Button
            variant="outline"
            size="sm"
            className="min-w-20"
            aria-label={t("projects.run.cancelLabel", { name: project.name })}
            onClick={run.cancel}
          >
            <X aria-hidden />
            {t("dashboard.projects.cancel")}
          </Button>
        ) : entry?.served || stopping ? (
          <Button
            variant="outline"
            size="sm"
            className="min-w-20"
            disabled={run.busy}
            aria-label={t("projects.run.stopLabel", { name: project.name })}
            onClick={run.stop}
          >
            {stopping ? (
              <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
            ) : (
              <Square aria-hidden />
            )}
            {t("dashboard.projects.stop")}
          </Button>
        ) : (
          <Button
            size="sm"
            className="min-w-20"
            disabled={run.busy || !canStart}
            title={
              entry && !canStart
                ? t(entry.state === "noPhp" ? "projects.site.noPhp" : "projects.site.folderMissing")
                : undefined
            }
            aria-label={t("projects.run.startLabel", { name: project.name })}
            onClick={run.start}
          >
            {run.busy ? (
              <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
            ) : (
              <Play aria-hidden />
            )}
            {t("dashboard.projects.start")}
          </Button>
        )}
      </div>
    </li>
  );
}
