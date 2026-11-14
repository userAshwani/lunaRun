import { ArrowLeft, FolderSearch, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link, useNavigate, useParams } from "react-router";
import type { Project } from "@/bindings/Project";
import type { ProjectDetection } from "@/bindings/ProjectDetection";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { useToast } from "@/components/toast-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useRuntimeJobEffects } from "@/features/runtimes/use-runtimes";
import { formatUnixDate } from "@/lib/format";
import { DatabaseRow, ExtensionsRow, RuntimeRow, WarningsList } from "./checklist";
import { ProjectSettings } from "./project-settings";
import { folderIsMissing, runtimeName } from "./project-format";
import {
  useProjectDetection,
  useProjects,
  useRemoveProject,
  useRescanWhenRuntimesChange,
} from "./use-projects";

function Facts({
  project,
  detection,
}: {
  project: Project;
  detection: ProjectDetection | undefined;
}) {
  const { t, i18n } = useTranslation();
  const framework = (detection?.project ?? project).framework;
  const report = detection?.report;
  const rows: [string, string][] = [
    [t("projects.detail.folder"), project.path],
    [
      t("projects.detail.framework"),
      framework
        ? framework.version
          ? t("projects.framework.laravel", { version: framework.version })
          : t("projects.framework.laravelNoVersion")
        : t("projects.framework.generic"),
    ],
    [
      t("projects.detail.docRoot"),
      report && !project.docRootCustom && !folderIsMissing(report)
        ? t("projects.detail.docRootEvidence", {
            path: project.docRoot,
            evidence: report.docRoot.evidence,
          })
        : project.docRoot,
    ],
    [t("projects.detail.packageManager"), report?.packageManager ?? t("projects.detail.none")],
  ];
  const scanned = project.lastDetection?.at;
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("projects.detail.aboutTitle")}</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-x-6 gap-y-3 text-sm sm:grid-cols-[10rem_1fr]">
          {rows.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="min-w-0 break-words">{value}</dd>
            </div>
          ))}
        </dl>
        {scanned !== undefined && (
          <p className="mt-4 text-[13px] text-muted-foreground">
            {t("projects.detail.lastScanned", { when: formatUnixDate(scanned, i18n.language) })}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function ChecklistSkeleton() {
  const { t } = useTranslation();
  return (
    <div role="status" aria-label={t("projects.detail.checklistLoading")} className="space-y-4">
      {[0, 1, 2, 3].map((row) => (
        <Skeleton key={row} className="h-10 w-full" />
      ))}
    </div>
  );
}

export function ProjectDetailPage() {
  const { t } = useTranslation();
  const { id = "" } = useParams();
  const navigate = useNavigate();
  const toast = useToast();
  const projects = useProjects();
  const project = projects.data?.find((candidate) => candidate.id === id);
  const scan = useProjectDetection(id, project !== undefined);
  const remove = useRemoveProject();
  const [confirmRemove, setConfirmRemove] = useState(false);
  useRescanWhenRuntimesChange();
  useRuntimeJobEffects(runtimeName);

  const back = (
    <Link
      to="/projects"
      className="mb-4 inline-flex items-center gap-1 rounded-sm text-sm text-muted-foreground hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
    >
      <ArrowLeft className="size-4" aria-hidden />
      {t("projects.detail.back")}
    </Link>
  );

  if (projects.isError) {
    return (
      <>
        {back}
        <ErrorState
          error={projects.error.error}
          title={t("projects.detail.loadError")}
          onRetry={() => {
            void projects.refetch();
          }}
        />
      </>
    );
  }
  if (!projects.data) {
    return (
      <>
        {back}
        <div role="status" aria-label={t("projects.loading")} className="space-y-4">
          <Skeleton className="h-8 w-64" />
          <Skeleton className="h-48 w-full" />
        </div>
      </>
    );
  }
  if (!project) {
    return (
      <>
        {back}
        <EmptyState
          icon={FolderSearch}
          title={t("projects.detail.notFoundTitle")}
          description={t("projects.detail.notFoundDescription")}
          action={
            <Button asChild variant="outline">
              <Link to="/projects">{t("projects.detail.back")}</Link>
            </Button>
          }
        />
      </>
    );
  }

  const detection = scan.data;
  const report = detection?.report;
  const resolution = detection?.resolution;

  return (
    <>
      {back}
      <PageHeader
        title={project.name}
        description={project.path}
        actions={
          <>
            <Button
              variant="outline"
              disabled={scan.isFetching}
              aria-label={t("projects.detail.rescanLabel", { name: project.name })}
              onClick={() => {
                void scan.refetch();
              }}
            >
              <RefreshCw
                className={scan.isFetching ? "animate-spin motion-reduce:animate-none" : undefined}
                aria-hidden
              />
              {scan.isFetching ? t("projects.detail.scanning") : t("projects.detail.rescan")}
            </Button>
            <Button
              variant="ghost"
              aria-label={t("projects.actions.removeLabel", { name: project.name })}
              onClick={() => {
                setConfirmRemove(true);
              }}
            >
              <Trash2 aria-hidden />
              {t("projects.actions.remove")}
            </Button>
          </>
        }
      />

      <div className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle>{t("projects.detail.checklistTitle")}</CardTitle>
          </CardHeader>
          <CardContent>
            {scan.isError && !detection ? (
              <ErrorState
                error={scan.error.error}
                title={t("projects.detail.scanError")}
                onRetry={() => {
                  void scan.refetch();
                }}
              />
            ) : !detection || !report || !resolution ? (
              <ChecklistSkeleton />
            ) : folderIsMissing(report) ? (
              // Nothing to resolve: a version here would only look reassuring.
              <p className="text-sm text-muted-foreground">{t("projects.detail.folderMissing")}</p>
            ) : (
              <ul className="divide-y">
                <RuntimeRow
                  runtime="php"
                  requirement={report.php}
                  resolution={resolution.php}
                  project={project}
                />
                <ExtensionsRow extensions={resolution.extensions} />
                {resolution.node && (
                  <RuntimeRow
                    runtime="node"
                    requirement={report.node}
                    resolution={resolution.node}
                    project={project}
                  />
                )}
                {resolution.database && (
                  <DatabaseRow suggestion={resolution.database} hint={report.database} />
                )}
              </ul>
            )}
          </CardContent>
        </Card>

        {report && report.warnings.length > 0 && (
          <Card>
            <CardHeader>
              <CardTitle>{t("projects.detail.warningsTitle")}</CardTitle>
            </CardHeader>
            <CardContent>
              <WarningsList warnings={report.warnings} />
            </CardContent>
          </Card>
        )}

        <Facts project={project} detection={detection} />
        <ProjectSettings
          key={`${project.id}:${project.name}:${project.docRoot}`}
          project={project}
          detection={detection}
        />
      </div>

      {confirmRemove && (
        <ConfirmDialog
          title={t("projects.remove.title", { name: project.name })}
          description={t("projects.remove.description", { path: project.path })}
          confirmLabel={t("projects.remove.confirm")}
          cancelLabel={t("projects.remove.cancel")}
          destructive
          busy={remove.isPending}
          onConfirm={() => {
            remove.mutate(project.id, {
              onSuccess: () => {
                toast.show({ title: t("projects.remove.done", { name: project.name }) });
                void navigate("/projects");
              },
              onError: () => {
                setConfirmRemove(false);
                toast.show({
                  tone: "info",
                  title: t("projects.remove.failed", { name: project.name }),
                });
              },
            });
          }}
          onCancel={() => {
            setConfirmRemove(false);
          }}
        />
      )}
    </>
  );
}
