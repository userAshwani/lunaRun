import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstalledView } from "@/bindings/InstalledView";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useErrorCategory } from "@/lib/errors";
import { RuntimeProblem } from "./runtime-problem";
import { useRuntimeUsage, useUninstallRuntime } from "./use-runtimes";

interface UninstallDialogProps {
  target: InstalledView;
  /** Display name of the runtime ("PHP"). */
  name: string;
  onClose: () => void;
}

function Notice({ title, hint, items }: { title: string; hint: string; items: string[] }) {
  return (
    <div className="rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
      <p className="font-medium">{title}</p>
      <ul className="mt-1 list-disc pl-5">
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
      <p className="mt-1 text-muted-foreground">{hint}</p>
    </div>
  );
}

/**
 * Confirms removing one installed version. It names what is affected: where the files are, the projects that
 * use the version, the services that must stop first and, for a database, whether to delete its data.
 */
export function UninstallDialog({ target, name, onClose }: UninstallDialogProps) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const usage = useRuntimeUsage(target.runtime, target.version);
  const uninstall = useUninstallRuntime();
  const [deleteData, setDeleteData] = useState(false);
  const who = { name, version: target.version };

  const report = usage.data;
  const blocked = (report?.runningServices.length ?? 0) > 0;
  const hasProjects = (report?.boundProjects.length ?? 0) > 0;
  const canDelete = report !== undefined && report.dataDirs.length > 0 && !report.dataShared;
  const willDelete = deleteData && canDelete;

  const confirmLabel = willDelete
    ? t("runtimes.uninstall.confirmAndDelete")
    : hasProjects
      ? t("runtimes.uninstall.confirmAnyway")
      : t("runtimes.uninstall.confirm");

  return (
    <ConfirmDialog
      title={t("runtimes.uninstall.title", who)}
      description={t("runtimes.uninstall.description")}
      confirmLabel={confirmLabel}
      cancelLabel={t("runtimes.uninstall.cancel")}
      destructive
      busy={uninstall.isPending}
      confirmDisabled={!report || blocked}
      onCancel={onClose}
      onConfirm={() => {
        uninstall.mutate(
          {
            runtime: target.runtime,
            version: target.version,
            deleteData: willDelete,
            confirmBoundProjects: hasProjects,
          },
          { onSuccess: onClose },
        );
      }}
    >
      <p className="rounded-md bg-muted px-3 py-2 text-xs">
        <span className="text-muted-foreground">{t("runtimes.uninstall.location")}: </span>
        <span className="font-mono break-all">{target.dir}</span>
      </p>

      {usage.isPending && (
        <div role="status" aria-label={t("runtimes.uninstall.loading", who)} className="space-y-2">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-2/3" />
        </div>
      )}

      {usage.isError && (
        <div role="alert" className="space-y-2 text-sm">
          <p className="font-medium">{t("runtimes.uninstall.loadError", who)}</p>
          <p className="text-muted-foreground">{categoryOf(usage.error.error.code)}</p>
          <p className="font-mono text-xs break-words text-muted-foreground">
            {usage.error.error.message}
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void usage.refetch();
            }}
          >
            {t("common.retry")}
          </Button>
        </div>
      )}

      {report && blocked && (
        <Notice
          title={t("runtimes.uninstall.servicesTitle")}
          hint={t("runtimes.uninstall.servicesHint", who)}
          items={report.runningServices}
        />
      )}
      {report && hasProjects && (
        <Notice
          title={t("runtimes.uninstall.projectsTitle")}
          hint={t("runtimes.uninstall.projectsHint", who)}
          items={report.boundProjects}
        />
      )}

      {report && report.dataDirs.length > 0 && (
        <div className="space-y-2 text-sm">
          <label className="flex items-start gap-2.5">
            <input
              type="checkbox"
              className="mt-0.5 size-4 accent-[var(--color-destructive)]"
              checked={willDelete}
              disabled={!canDelete}
              onChange={(event) => {
                setDeleteData(event.target.checked);
              }}
            />
            <span className="font-medium">{t("runtimes.uninstall.data.label")}</span>
          </label>
          <p className="text-muted-foreground">{t("runtimes.uninstall.data.where")}</p>
          <ul className="space-y-0.5 font-mono text-xs break-all">
            {report.dataDirs.map((dir) => (
              <li key={dir}>{dir}</li>
            ))}
          </ul>
          <p className="text-muted-foreground">
            {report.dataShared
              ? t("runtimes.uninstall.data.shared")
              : willDelete
                ? t("runtimes.uninstall.data.warning")
                : t("runtimes.uninstall.data.kept")}
          </p>
        </div>
      )}

      {uninstall.isError && (
        <RuntimeProblem
          error={uninstall.error.error}
          name={name}
          version={target.version}
          action="uninstall"
        />
      )}
    </ConfirmDialog>
  );
}
