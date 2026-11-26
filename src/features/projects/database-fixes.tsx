import { CircleAlert, Database, FilePen, LoaderCircle } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { DbHint } from "@/bindings/DbHint";
import type { LunaError } from "@/bindings/LunaError";
import type { PortChange } from "@/bindings/PortChange";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { useToast } from "@/components/toast-context";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useErrorCategory } from "@/lib/errors";
import { IpcError, toLunaError } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import {
  useApplyEnvPort,
  useCreateDatabase,
  useDatabaseStatus,
  useEnvPortChange,
} from "./use-database-fixes";

function errorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

function reasonOf(error: LunaError): string | null {
  const reason = error.details?.reason;
  return typeof reason === "string" ? reason : null;
}

/** A problem with a fix, in one block: what happened, what to do, the technical message, and a way to try again. */
function FixProblem({
  title,
  advice,
  error,
  onRetry,
}: {
  title: string;
  advice: string | null;
  error: LunaError;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{title}</p>
        <p className="text-[13px] text-muted-foreground">{advice ?? categoryOf(error.code)}</p>
        <p className="mt-1 font-mono text-xs break-words text-muted-foreground">{error.message}</p>
      </div>
      {onRetry && (
        <Button variant="outline" size="sm" onClick={onRetry}>
          {t("projects.fixes.retry")}
        </Button>
      )}
    </div>
  );
}

/** One line of the preview of a change to `.env`: `-` for what goes, `+` for what comes. */
function DiffLine({ kind, text }: { kind: "removed" | "added"; text: string }) {
  const { t } = useTranslation();
  return (
    <div
      className={cn(
        "flex gap-2 px-3 py-1.5",
        kind === "removed" ? "bg-destructive/10" : "bg-success/10",
      )}
    >
      <span aria-hidden>{kind === "removed" ? "-" : "+"}</span>
      <span className="sr-only">
        {kind === "removed" ? t("projects.fixes.port.removed") : t("projects.fixes.port.added")}
      </span>
      <code className="min-w-0 break-all">{text}</code>
    </div>
  );
}

/** What the page tells about a `.env` that lunaRun cannot (or will not) edit, or that changed under the preview. */
function envAdvice(error: LunaError, t: ReturnType<typeof useTranslation>["t"]): string | null {
  if (error.code !== "ENV_CHANGED") return null;
  switch (reasonOf(error)) {
    case "link":
      return t("projects.fixes.port.cannot.link");
    case "notFile":
      return t("projects.fixes.port.cannot.notFile");
    case "tooLarge":
      return t("projects.fixes.port.cannot.tooLarge");
    case "notUtf8":
      return t("projects.fixes.port.cannot.notUtf8");
    default:
      return t("projects.fixes.port.changed");
  }
}

function PortFix({ projectId }: { projectId: string }) {
  const { t } = useTranslation();
  const toast = useToast();
  const query = useEnvPortChange(projectId);
  const apply = useApplyEnvPort(projectId);
  const [previewing, setPreviewing] = useState<PortChange | null>(null);

  if (query.isPending)
    return <Skeleton className="h-5 w-72" aria-label={t("projects.fixes.checking")} />;
  if (query.isError) {
    const error = errorOf(query.error);
    return (
      <FixProblem
        title={t("projects.fixes.port.checkFailed")}
        advice={envAdvice(error, t)}
        error={error}
        onRetry={() => {
          void query.refetch();
        }}
      />
    );
  }
  const change = query.data;
  const applyError = apply.isError ? errorOf(apply.error) : null;
  if (!change && !applyError) return null;

  return (
    <div className="space-y-2">
      {change && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <p className="min-w-0 flex-1 text-[13px] text-warning-soft-foreground">
            {change.before === null
              ? t("projects.fixes.port.missingLine", { port: change.port })
              : t("projects.fixes.port.differs", { port: change.port })}
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              apply.reset();
              setPreviewing(change);
            }}
          >
            <FilePen aria-hidden />
            {t("projects.fixes.port.button")}
          </Button>
        </div>
      )}
      {applyError && (
        <FixProblem
          title={t("projects.fixes.port.applyFailed")}
          advice={envAdvice(applyError, t)}
          error={applyError}
        />
      )}
      {previewing && (
        <ConfirmDialog
          title={t("projects.fixes.port.dialogTitle")}
          description={t("projects.fixes.port.dialogText", { backup: previewing.backup })}
          confirmLabel={t("projects.fixes.port.confirm")}
          cancelLabel={t("projects.fixes.port.cancel")}
          busy={apply.isPending}
          onCancel={() => {
            setPreviewing(null);
          }}
          onConfirm={() => {
            apply.mutate(previewing, {
              onSuccess: (done) => {
                toast.show({
                  title: t("projects.fixes.port.done", { backup: done.backup }),
                  tone: "success",
                });
              },
              onSettled: () => {
                setPreviewing(null);
              },
            });
          }}
        >
          <div
            role="group"
            aria-label={t("projects.fixes.port.diffLabel")}
            className="overflow-hidden rounded-md border font-mono text-xs"
          >
            <p className="bg-muted px-3 py-1.5 text-muted-foreground">
              {t("projects.fixes.port.file", { line: previewing.line })}
            </p>
            {previewing.before !== null && <DiffLine kind="removed" text={previewing.before} />}
            <DiffLine kind="added" text={previewing.after} />
          </div>
        </ConfirmDialog>
      )}
    </div>
  );
}

function CreateDatabaseFix({ projectId, name }: { projectId: string; name: string | null }) {
  const { t } = useTranslation();
  const toast = useToast();
  const status = useDatabaseStatus(projectId);
  const create = useCreateDatabase(projectId);

  if (status.isPending)
    return <Skeleton className="h-5 w-64" aria-label={t("projects.fixes.checking")} />;
  if (status.isError) {
    return (
      <FixProblem
        title={t("projects.fixes.database.checkFailed")}
        advice={null}
        error={errorOf(status.error)}
        onRetry={() => {
          void status.refetch();
        }}
      />
    );
  }
  const shown = name ?? "";
  const createError = create.isError ? errorOf(create.error) : null;
  switch (status.data) {
    case "missing":
      return (
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            <p className="min-w-0 flex-1 text-[13px] text-warning-soft-foreground">
              {t("projects.fixes.database.missing", { name: shown })}
            </p>
            <Button
              variant="outline"
              size="sm"
              disabled={create.isPending}
              onClick={() => {
                create.mutate(undefined, {
                  onSuccess: (done) => {
                    toast.show({
                      title: t(
                        done.created
                          ? "projects.fixes.database.created"
                          : "projects.fixes.database.alreadyThere",
                        { name: done.name },
                      ),
                      tone: "success",
                    });
                  },
                });
              }}
            >
              {create.isPending ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
              ) : (
                <Database aria-hidden />
              )}
              {t("projects.fixes.database.create")}
            </Button>
          </div>
          {createError && (
            <FixProblem
              title={t("projects.fixes.database.createFailed")}
              advice={
                createError.code === "DATABASE_NOT_RUNNING"
                  ? t("projects.fixes.database.notRunningError")
                  : null
              }
              error={createError}
            />
          )}
        </div>
      );
    case "exists":
      return (
        <p className="text-[13px] text-muted-foreground">
          {t("projects.fixes.database.exists", { name: shown })}
        </p>
      );
    case "notRunning":
      return name ? (
        <p className="text-[13px] text-muted-foreground">
          {t("projects.fixes.database.notRunning", { name })}
        </p>
      ) : null;
    case "invalidName":
      return (
        <p className="text-[13px] text-warning-soft-foreground">
          {t("projects.fixes.database.invalidName", { name: shown })}
        </p>
      );
    default:
      return null;
  }
}

/**
 * The two fixes the database row of the checklist can offer (FR-PJ-05): point `DB_PORT` of the `.env` at the port lunaRun
 * runs the database on (a preview of the one line, a backup, then the edit), and create the database the `.env` names.
 * Each shows nothing when there is nothing to fix.
 */
export function DatabaseFixes({ projectId, hint }: { projectId: string; hint: DbHint | null }) {
  // A project with no `.env` of its own has nothing lunaRun could edit; the example file is never touched.
  return (
    <div className="space-y-2 pt-1">
      {hint?.file === ".env" && <PortFix projectId={projectId} />}
      {hint?.database ? <CreateDatabaseFix projectId={projectId} name={hint.database} /> : null}
    </div>
  );
}
