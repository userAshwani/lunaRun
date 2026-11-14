import { FolderKanban, FolderPlus, LoaderCircle, Upload } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import type { LunaError } from "@/bindings/LunaError";
import type { Project } from "@/bindings/Project";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { useToast } from "@/components/toast-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { IpcError, toLunaError } from "@/lib/ipc";
import { pickFolder } from "@/lib/native";
import { AddProblem } from "./add-problem";
import { ProjectsTable, type ScanView } from "./projects-table";
import { useFolderDrop } from "./use-folder-drop";
import {
  useAddProject,
  useProjectDetections,
  useProjects,
  useRemoveProject,
  useRescanWhenRuntimesChange,
} from "./use-projects";

function errorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

function ProjectsSkeleton() {
  const { t } = useTranslation();
  return (
    <Card>
      <CardContent>
        <div role="status" aria-label={t("projects.loading")} className="space-y-3">
          {[0, 1, 2].map((row) => (
            <Skeleton key={row} className="h-10 w-full" />
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

/** Covers the window while a folder is dragged over it, so it is clear where to let go. */
function DropOverlay() {
  const { t } = useTranslation();
  return (
    <div
      role="status"
      className="pointer-events-none fixed inset-0 z-40 flex items-center justify-center bg-background/80 p-8"
    >
      <div className="flex max-w-md flex-col items-center rounded-xl border-2 border-dashed border-primary bg-card px-10 py-12 text-center shadow-sm">
        <div className="flex size-12 items-center justify-center rounded-full bg-primary-soft text-primary-soft-foreground">
          <Upload className="size-6" aria-hidden />
        </div>
        <p className="mt-4 text-base font-semibold">{t("projects.add.dropTitle")}</p>
        <p className="mt-1 text-sm text-muted-foreground">{t("projects.add.dropHint")}</p>
      </div>
    </div>
  );
}

export function ProjectsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const toast = useToast();
  const projects = useProjects();
  const scans = useProjectDetections(projects.data);
  const add = useAddProject();
  const remove = useRemoveProject();
  const [problem, setProblem] = useState<{ error: LunaError; path: string } | null>(null);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<Project | null>(null);
  useRescanWhenRuntimesChange();

  const addPaths = async (paths: readonly string[]) => {
    if (paths.length === 0) return;
    setAdding(true);
    setProblem(null);
    let lastAdded: Project | null = null;
    for (const path of paths) {
      try {
        const added = await add.mutateAsync(path);
        lastAdded = added.project;
        toast.show({ title: t("projects.add.added", { name: added.project.name }) });
      } catch (thrown) {
        setProblem({ error: errorOf(thrown), path });
      }
    }
    setAdding(false);
    // One folder added: show what it needs straight away.
    if (paths.length === 1 && lastAdded) void navigate(`/projects/${lastAdded.id}`);
  };

  const choose = async () => {
    try {
      const chosen = await pickFolder(t("projects.add.dialogTitle"));
      if (chosen) await addPaths([chosen]);
    } catch (thrown) {
      setProblem({ error: errorOf(thrown), path: "" });
    }
  };

  const hovering = useFolderDrop((paths) => {
    void addPaths(paths);
  });

  const scanOf = (id: string): ScanView => {
    const index = projects.data?.findIndex((project) => project.id === id) ?? -1;
    const scan = index >= 0 ? scans[index] : undefined;
    if (scan?.data) return { status: "ready", detection: scan.data };
    return scan?.isError ? { status: "error" } : { status: "loading" };
  };

  const addButton = (
    <Button
      onClick={() => {
        void choose();
      }}
      disabled={adding}
    >
      {adding ? (
        <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
      ) : (
        <FolderPlus aria-hidden />
      )}
      {t("projects.add.button")}
    </Button>
  );

  return (
    <>
      <PageHeader
        title={t("nav.projects")}
        description={t("pages.projects.description")}
        actions={projects.data && projects.data.length > 0 ? addButton : undefined}
      />

      {projects.isError ? (
        <ErrorState
          error={projects.error.error}
          title={t("projects.loadError")}
          onRetry={() => {
            void projects.refetch();
          }}
        />
      ) : !projects.data ? (
        <ProjectsSkeleton />
      ) : (
        <div className="space-y-6">
          {problem && (
            <AddProblem
              error={problem.error}
              path={problem.path}
              onDismiss={() => {
                setProblem(null);
              }}
            />
          )}
          {projects.data.length === 0 ? (
            <EmptyState
              icon={FolderKanban}
              title={t("projects.empty.title")}
              description={t("projects.empty.description")}
              action={addButton}
            />
          ) : (
            <Card>
              <CardContent>
                <ProjectsTable projects={projects.data} scanOf={scanOf} onRemove={setRemoving} />
              </CardContent>
            </Card>
          )}
        </div>
      )}

      {hovering && <DropOverlay />}
      {removing && (
        <ConfirmDialog
          title={t("projects.remove.title", { name: removing.name })}
          description={t("projects.remove.description", { path: removing.path })}
          confirmLabel={t("projects.remove.confirm")}
          cancelLabel={t("projects.remove.cancel")}
          destructive
          busy={remove.isPending}
          onConfirm={() => {
            const target = removing;
            remove.mutate(target.id, {
              onSuccess: () => {
                toast.show({ title: t("projects.remove.done", { name: target.name }) });
                setRemoving(null);
              },
              onError: (thrown) => {
                setRemoving(null);
                setProblem({ error: errorOf(thrown), path: target.path });
              },
            });
          }}
          onCancel={() => {
            setRemoving(null);
          }}
        />
      )}
    </>
  );
}
