import type { UseQueryResult } from "@tanstack/react-query";
import { PackageOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { InstalledView } from "@/bindings/InstalledView";
import { ErrorState } from "@/components/error-state";
import { Badge } from "@/components/ui/badge";
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

interface InstalledCardProps {
  runtimeId: string;
  name: string;
  /** Owned by the page, which also needs it for the tab counts: one request, one error to show. */
  query: UseQueryResult<InstalledView[]>;
}

export function InstalledCard({ runtimeId, name, query }: InstalledCardProps) {
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
          />
        )}
      </CardContent>
    </Card>
  );
}

function InstalledTable({ rows, name }: { rows: readonly InstalledView[]; name: string }) {
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
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((entry) => (
            <TableRow key={`${entry.runtime}@${entry.version}`}>
              <TableCell className="font-medium whitespace-nowrap tabular-nums">
                {entry.version}
                {entry.rosetta && <Badge className="ml-2">{t("runtimes.installed.rosetta")}</Badge>}
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
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableScroll>
  );
}
