import { CircleAlert, LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { SiteEntry } from "@/bindings/SiteEntry";
import type { SitesView } from "@/bindings/SitesView";
import { CopyButton } from "@/components/copy-button";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useErrorCategory } from "@/lib/errors";

interface SiteFactProps {
  /** The answer of `sites_list`, while it is there. */
  view: SitesView | undefined;
  failed: boolean;
  projectId: string;
}

/**
 * The site address of a project, with the plain truth about whether anything answers on it: the web server may not be
 * installed or not running, and PHP pages also need the PHP service. A project with no site says why.
 */
export function SiteFact({ view, failed, projectId }: SiteFactProps) {
  const { t } = useTranslation();
  if (failed) return <span className="text-muted-foreground">{t("projects.site.unknown")}</span>;
  if (!view) return <Skeleton className="h-5 w-48" aria-label={t("projects.site.loading")} />;
  const entry: SiteEntry | undefined = view.sites.find((site) => site.projectId === projectId);
  if (!entry) return <span className="text-muted-foreground">{t("projects.site.unknown")}</span>;
  if (entry.state === "noPhp") {
    return <span className="text-muted-foreground">{t("projects.site.noPhp")}</span>;
  }
  if (entry.state === "folderMissing") {
    return <span className="text-muted-foreground">{t("projects.site.folderMissing")}</span>;
  }
  const web =
    view.webServer === "notInstalled"
      ? "webNotInstalled"
      : view.webServer === "stopped"
        ? "webStopped"
        : "webRunning";
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono tabular-nums">{entry.address}</span>
        <CopyButton
          text={entry.address ?? ""}
          label={t("projects.site.copy")}
          doneLabel={t("projects.site.copied")}
          failedLabel={t("projects.site.copyFailed")}
          size="sm"
        />
      </div>
      <p className="text-[13px] text-muted-foreground">{t(`projects.site.${web}`)}</p>
      {entry.phpVersion && (
        <p className="text-[13px] text-muted-foreground">
          {t("projects.site.phpHint", { version: entry.phpVersion })}
        </p>
      )}
    </div>
  );
}

interface SitesProblemProps {
  view: SitesView | undefined;
  retrying: boolean;
  onRetry: () => void;
}

/** Caddy refused the latest change: what it said, and a way to try again. Nothing when there is no problem. */
export function SitesProblem({ view, retrying, onRetry }: SitesProblemProps) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const problem = view?.problem;
  if (!problem) return null;
  // The two things Caddy can refuse have their own sentence; anything else is named by its category.
  const known =
    problem.code === "CONFIG_INVALID"
      ? t("projects.site.problem.CONFIG_INVALID")
      : problem.code === "RELOAD_FAILED"
        ? t("projects.site.problem.RELOAD_FAILED")
        : null;
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-xl border border-destructive/30 bg-destructive/5 px-4 py-3"
    >
      <CircleAlert className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{t("projects.site.problem.title")}</p>
        <p className="mt-1 text-[13px] text-muted-foreground">
          {known ?? `${t("projects.site.problem.other")} ${categoryOf(problem.code)}.`}
        </p>
        <p className="mt-2 rounded-md bg-muted px-3 py-2 font-mono text-xs break-words text-muted-foreground">
          {problem.message}
        </p>
      </div>
      <Button variant="outline" size="sm" disabled={retrying} onClick={onRetry}>
        {retrying && (
          <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
        )}
        {t("projects.site.problem.retry")}
      </Button>
    </div>
  );
}
