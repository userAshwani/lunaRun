import { ArrowRight, Trash2, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { Project } from "@/bindings/Project";
import type { ProjectDetection } from "@/bindings/ProjectDetection";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableScroll,
} from "@/components/ui/table";
import { folderIsMissing, runtimeState } from "./project-format";
import { RuntimeStateBadge } from "./runtime-state-badge";

/** Where a project's scan is: still being read, could not be read, or done. */
export type ScanView =
  { status: "loading" } | { status: "error" } | { status: "ready"; detection: ProjectDetection };

interface ProjectsTableProps {
  projects: readonly Project[];
  scanOf: (id: string) => ScanView;
  onRemove: (project: Project) => void;
}

function useFrameworkLabel() {
  const { t } = useTranslation();
  return (project: Project): string => {
    const framework = project.framework;
    if (!framework) return t("projects.framework.generic");
    return framework.version
      ? t("projects.framework.laravel", { version: framework.version })
      : t("projects.framework.laravelNoVersion");
  };
}

/** A runtime cell: a skeleton while the scan runs, a short note when it failed, else the badge. */
function RuntimeCell({
  runtime,
  scan,
  missing,
}: {
  runtime: "php" | "node";
  scan: ScanView;
  missing: boolean;
}) {
  const { t } = useTranslation();
  // A folder that is gone has nothing to resolve: a version here would only look reassuring.
  if (missing) {
    return (
      <span className="text-[13px] text-muted-foreground">{t("projects.runtime.notChecked")}</span>
    );
  }
  if (scan.status === "loading") {
    return <Skeleton className="h-5 w-28" aria-label={t("projects.runtime.checking")} />;
  }
  if (scan.status === "error") {
    return (
      <span className="text-[13px] text-muted-foreground">{t("projects.runtime.unreadable")}</span>
    );
  }
  const resolution =
    runtime === "php" ? scan.detection.resolution.php : scan.detection.resolution.node;
  return <RuntimeStateBadge runtime={runtime} state={runtimeState(resolution)} />;
}

export function ProjectsTable({ projects, scanOf, onRemove }: ProjectsTableProps) {
  const { t } = useTranslation();
  const frameworkLabel = useFrameworkLabel();

  return (
    // The table is the page, not one card among several: it takes the height it needs and the page scrolls.
    <TableScroll label={t("projects.table.label")} className="max-h-none">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead>{t("projects.table.project")}</TableHead>
            <TableHead>{t("projects.table.php")}</TableHead>
            <TableHead>{t("projects.table.node")}</TableHead>
            <TableHead className="text-right">
              <span className="sr-only">{t("projects.table.actions")}</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {projects.map((project) => {
            const scan = scanOf(project.id);
            const missing = scan.status === "ready" && folderIsMissing(scan.detection.report);
            return (
              <TableRow key={project.id}>
                <TableCell className="max-w-[13rem] align-top">
                  <p
                    className="truncate"
                    title={`${project.name} · ${frameworkLabel(scan.status === "ready" ? scan.detection.project : project)}`}
                  >
                    <Link
                      to={`/projects/${project.id}`}
                      className="font-medium hover:underline focus-visible:underline"
                    >
                      {project.name}
                    </Link>
                    <span className="text-[13px] text-muted-foreground">
                      {" "}
                      · {frameworkLabel(scan.status === "ready" ? scan.detection.project : project)}
                    </span>
                  </p>
                  <p className="truncate text-[13px] text-muted-foreground" title={project.path}>
                    {project.path}
                  </p>
                  {missing && (
                    <Badge variant="destructive" className="mt-1">
                      <TriangleAlert className="size-3" aria-hidden />
                      {t("projects.folderMissing")}
                    </Badge>
                  )}
                </TableCell>
                <TableCell className="align-top">
                  <RuntimeCell runtime="php" scan={scan} missing={missing} />
                </TableCell>
                <TableCell className="align-top">
                  <RuntimeCell runtime="node" scan={scan} missing={missing} />
                </TableCell>
                <TableCell className="align-top">
                  <div className="flex justify-end gap-2">
                    <Button
                      asChild
                      variant="outline"
                      size="icon"
                      className="size-8"
                      aria-label={t("projects.actions.openLabel", { name: project.name })}
                      title={t("projects.actions.open")}
                    >
                      <Link to={`/projects/${project.id}`}>
                        <ArrowRight aria-hidden />
                      </Link>
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-8"
                      aria-label={t("projects.actions.removeLabel", { name: project.name })}
                      title={t("projects.actions.remove")}
                      onClick={() => {
                        onRemove(project);
                      }}
                    >
                      <Trash2 aria-hidden />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </TableScroll>
  );
}
