import { RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstalledView } from "@/bindings/InstalledView";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { Tabs } from "@/components/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { AvailableCard } from "./available-card";
import { InstalledCard } from "./installed-card";
import { RegistryStatusLine } from "./registry-status";
import { orderRuntimes } from "./runtime-order";
import { useInstalledRuntimes, useRegistryRefresh, useRuntimesAvailable } from "./use-runtimes";

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

export function RuntimesPage() {
  const { t } = useTranslation();
  const available = useRuntimesAvailable();
  const installed = useInstalledRuntimes();
  const refresh = useRegistryRefresh();
  const [selected, setSelected] = useState<string | null>(null);

  const checking = refresh.isPending || available.data?.registry.refreshing === true;
  const catalog = available.data
    ? withUnlistedInstalls(orderRuntimes(available.data.runtimes), installed.data)
    : [];
  const current = catalog.find((entry) => entry.id === selected) ?? catalog[0];

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
            <InstalledCard runtimeId={current.id} name={current.displayName} query={installed} />
            <AvailableCard key={current.id} catalog={current} installed={installed.data} />
          </Tabs>
        </>
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
