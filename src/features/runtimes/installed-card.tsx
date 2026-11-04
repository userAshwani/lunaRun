import type { UseQueryResult } from "@tanstack/react-query";
import { LoaderCircle, PackageOpen, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { InstalledView } from "@/bindings/InstalledView";
import type { JobInfo } from "@/bindings/JobInfo";
import { ErrorState } from "@/components/error-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
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
import { formatUnixDate } from "@/lib/format";
import { jobKey } from "./runtime-jobs";

interface InstalledCardProps {
  runtimeId: string;
  name: string;
  /** Owned by the page, which also needs it for the tab counts: one request, one error to show. */
  query: UseQueryResult<InstalledView[]>;
  /** The newest install or uninstall job of each version, by job key. */
  jobs: ReadonlyMap<string, JobInfo>;
  onUninstall: (entry: InstalledView) => void;
}

export function InstalledCard({ runtimeId, name, query, jobs, onUninstall }: InstalledCardProps) {
  const { t } = useTranslation();

  return (
    <Card className="mb-6">
      <CardHeader>
        <CardTitle>{t("runtimes.installed.title")}</CardTitle>
        <CardDescription>{t("runtimes.installed.description", { name })}</CardDescription>
      </CardHeader>
      <CardContent>
        {query.isError ? (
          <ErrorState
            error={query.error.error}
            title={t("runtimes.installed.loadError")}
            onRetry={() => {
              void query.refetch();
            }}
          />
        ) : !query.data ? (
          <div role="status" aria-label={t("runtimes.installed.loading")} className="space-y-3">
            <Skeleton className="h-5 w-full" />
            <Skeleton className="h-5 w-2/3" />
          </div>
        ) : (
          <InstalledTable
            rows={query.data.filter((entry) => entry.runtime === runtimeId)}
            name={name}
            jobs={jobs}
            onUninstall={onUninstall}
          />
        )}
      </CardContent>
    </Card>
  );
}

interface InstalledTableProps {
  rows: readonly InstalledView[];
  name: string;
  jobs: ReadonlyMap<string, JobInfo>;
  onUninstall: (entry: InstalledView) => void;
}

function InstalledTable({ rows, name, jobs, onUninstall }: InstalledTableProps) {
  const { t, i18n } = useTranslation();
  if (rows.length === 0) {
    // Compact on purpose: this sits inside a card, and the list of versions to install is right below.
    return (
      <div className="flex items-center gap-4 rounded-lg border border-dashed px-5 py-6">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-full bg-primary-soft text-primary-soft-foreground">
          <PackageOpen className="size-5" aria-hidden />
        </div>
        <div className="min-w-0">
          <p className="font-medium">{t("runtimes.installed.emptyTitle", { name })}</p>
          <p className="text-sm text-muted-foreground">{t("runtimes.installed.empty", { name })}</p>
        </div>
      </div>
    );
  }
  return (
    <TableScroll label={t("runtimes.installed.title")}>
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead>{t("runtimes.installed.columns.version")}</TableHead>
            <TableHead>{t("runtimes.installed.columns.installedOn")}</TableHead>
            <TableHead>{t("runtimes.installed.columns.location")}</TableHead>
            <TableHead className="text-right">
              <span className="sr-only">{t("runtimes.installed.columns.actions")}</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((entry) => {
            const removing = jobs.get(jobKey(entry.runtime, entry.version))?.state === "running";
            const who = { name, version: entry.version };
            return (
              <TableRow key={`${entry.runtime}@${entry.version}`}>
                <TableCell className="font-medium whitespace-nowrap tabular-nums">
                  {entry.version}
                  {entry.rosetta && (
                    <Badge className="ml-2">{t("runtimes.installed.rosetta")}</Badge>
                  )}
                </TableCell>
                <TableCell className="whitespace-nowrap text-muted-foreground tabular-nums">
                  {formatUnixDate(entry.installedAt, i18n.language)}
                </TableCell>
                <TableCell
                  className="max-w-0 truncate font-mono text-xs text-muted-foreground"
                  title={entry.dir}
                >
                  {entry.dir}
                </TableCell>
                <TableCell className="text-right">
                  {removing ? (
                    <span
                      className="inline-flex items-center gap-1.5 text-[13px] text-muted-foreground"
                      aria-label={t("runtimes.actions.removingLabel", who)}
                    >
                      <LoaderCircle
                        className="size-4 animate-spin motion-reduce:animate-none"
                        aria-hidden
                      />
                      {t("runtimes.actions.removing")}
                    </span>
                  ) : (
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={t("runtimes.actions.uninstallLabel", who)}
                      onClick={() => {
                        onUninstall(entry);
                      }}
                    >
                      <Trash2 aria-hidden />
                      {t("runtimes.actions.uninstall")}
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </TableScroll>
  );
}
