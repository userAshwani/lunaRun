import { RefreshCw } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstalledView } from "@/bindings/InstalledView";
import type { LunaError } from "@/bindings/LunaError";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { Tabs } from "@/components/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useCancelJob, useJobs } from "@/features/jobs/use-jobs";
import { AvailableCard } from "./available-card";
import { InstalledCard } from "./installed-card";
import { RegistryStatusLine } from "./registry-status";
import { orderRuntimes } from "./runtime-order";
import { jobKind, latestRuntimeJobs, parseJobKey, type JobKind } from "./runtime-jobs";
import { RuntimeProblem } from "./runtime-problem";
import { UninstallDialog } from "./uninstall-dialog";
import {
  useInstallRuntime,
  useInstalledRuntimes,
  useRegistryRefresh,
  useRuntimeJobEffects,
  useRuntimesAvailable,
} from "./use-runtimes";

function RuntimesSkeleton() {
  const { t } = useTranslation();
  return (
    <div role="status" aria-label={t("runtimes.loading")} className="space-y-6">
      <Skeleton className="h-9 w-80" />
      <Card>
        <CardContent className="space-y-3">
          {[0, 1, 2, 3].map((row) => (
            <Skeleton key={row} className="h-6 w-full" />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

/** Something that went wrong with one install or uninstall and is still on screen. */
interface Problem {
  id: string;
  runtime: string;
  version: string;
  action: JobKind;
  error: LunaError;
}

export function RuntimesPage() {
  const { t } = useTranslation();
  const available = useRuntimesAvailable();
  const installed = useInstalledRuntimes();
  const refresh = useRegistryRefresh();
  const jobs = useJobs();
  const install = useInstallRuntime();
  const cancel = useCancelJob();
  const [selected, setSelected] = useState<string | null>(null);
  const [uninstalling, setUninstalling] = useState<InstalledView | null>(null);
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  const [refused, setRefused] = useState<readonly Problem[]>([]);

  const checking = refresh.isPending || available.data?.registry.refreshing === true;
  const catalog = available.data
    ? withUnlistedInstalls(orderRuntimes(available.data.runtimes), installed.data)
    : [];
  const current = catalog.find((entry) => entry.id === selected) ?? catalog[0];
  const nameOf = (runtime: string) =>
    catalog.find((entry) => entry.id === runtime)?.displayName ?? runtime;

  useRuntimeJobEffects(nameOf);

  const latest = useMemo(() => latestRuntimeJobs(jobs.data ?? []), [jobs.data]);

  // An attempt that failed stays on screen until dismissed or until the next attempt replaces it.
  const problems: Problem[] = [...refused];
  for (const job of latest.values()) {
    const parts = parseJobKey(job.key);
    const action = jobKind(job);
    if (job.state !== "failed" || !job.error || !parts || !action || dismissed.has(job.id))
      continue;
    problems.push({ id: job.id, ...parts, action, error: job.error });
  }

  const dismiss = (problem: Problem) => {
    setRefused((current) => current.filter((entry) => entry.id !== problem.id));
    setDismissed((current) => new Set(current).add(problem.id));
  };

  const startInstall = (runtime: string, version: string) => {
    install.mutate(
      { runtime, version },
      {
        onError: (thrown) => {
          setRefused((current) => [
            ...current,
            {
              id: `refused-${runtime}@${version}-${String(Date.now())}`,
              runtime,
              version,
              action: "install",
              error: thrown.error,
            },
          ]);
        },
      },
    );
  };

  return (
    <>
      <PageHeader
        title={t("nav.runtimes")}
        description={t("pages.runtimes.description")}
        actions={
          <Button
            variant="outline"
            disabled={checking || !available.data}
            onClick={() => {
              refresh.mutate();
            }}
          >
            <RefreshCw className={checking ? "animate-spin" : undefined} aria-hidden />
            {checking ? t("runtimes.refresh.running") : t("runtimes.refresh.action")}
          </Button>
        }
      />

      {available.isError ? (
        <ErrorState
          error={available.error.error}
          title={t("runtimes.loadError")}
          onRetry={() => {
            void available.refetch();
          }}
        />
      ) : !available.data || !current ? (
        <RuntimesSkeleton />
      ) : (
        <>
          <RegistryStatusLine status={available.data.registry} />
          {refresh.isError && (
            <p role="alert" className="mb-6 text-[13px] text-destructive">
              {t("runtimes.refresh.failed")}: {refresh.error.error.message}
            </p>
          )}
          {jobs.isError && (
            <p role="alert" className="mb-6 flex items-center gap-3 text-[13px] text-destructive">
              {t("runtimes.jobsError")}
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  void jobs.refetch();
                }}
              >
                {t("common.retry")}
              </Button>
            </p>
          )}
          {problems.length > 0 && (
            <div className="mb-6 space-y-3">
              {problems.map((problem) => (
                <RuntimeProblem
                  key={problem.id}
                  error={problem.error}
                  name={nameOf(problem.runtime)}
                  version={problem.version}
                  action={problem.action}
                  onDismiss={() => {
                    dismiss(problem);
                  }}
                />
              ))}
            </div>
          )}
          <Tabs
            label={t("runtimes.tabsLabel")}
            value={current.id}
            onChange={setSelected}
            items={catalog.map((entry) => ({
              id: entry.id,
              label: entry.displayName,
              trailing: <InstalledCount runtimeId={entry.id} installed={installed.data} />,
            }))}
          >
            <InstalledCard
              runtimeId={current.id}
              name={current.displayName}
              query={installed}
              jobs={latest}
              onUninstall={setUninstalling}
            />
            <AvailableCard
              key={current.id}
              catalog={current}
              installed={installed.data}
              jobs={latest}
              starting={install.isPending}
              onInstall={(version) => {
                startInstall(current.id, version);
              }}
              onCancel={(jobId) => {
                cancel.mutate(jobId);
              }}
            />
          </Tabs>
        </>
      )}

      {uninstalling && (
        <UninstallDialog
          target={uninstalling}
          name={nameOf(uninstalling.runtime)}
          onClose={() => {
            setUninstalling(null);
          }}
        />
      )}
    </>
  );
}

/** A small count on a tab, only when something of that runtime is installed. */
function InstalledCount({
  runtimeId,
  installed,
}: {
  runtimeId: string;
  installed: readonly InstalledView[] | undefined;
}) {
  const count = (installed ?? []).filter((entry) => entry.runtime === runtimeId).length;
  return count > 0 ? <Badge variant="success">{count}</Badge> : null;
}

/**
 * An installed runtime the version list no longer knows (removed upstream, or a list that is older than
 * the install) still gets its tab, so the user can see and later remove it.
 */
function withUnlistedInstalls(
  catalog: RuntimeCatalog[],
  installed: readonly InstalledView[] | undefined,
): RuntimeCatalog[] {
  const known = new Set(catalog.map((entry) => entry.id));
  const extra = [...new Set((installed ?? []).map((entry) => entry.runtime))]
    .filter((id) => !known.has(id))
    .map((id): RuntimeCatalog => ({ id, displayName: id, license: null, versions: [] }));
  return [...catalog, ...extra];
}
