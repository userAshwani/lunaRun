import { Boxes } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstalledView } from "@/bindings/InstalledView";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import { EmptyState } from "@/components/empty-state";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableScroll,
} from "@/components/ui/table";
import { formatBytes, formatIsoDate } from "@/lib/format";
import { VersionBadges } from "./version-badges";

export function AvailableCard({
  catalog,
  installed,
}: {
  catalog: RuntimeCatalog;
  installed: readonly InstalledView[] | undefined;
}) {
  const { t, i18n } = useTranslation();
  const [showEol, setShowEol] = useState(false);
  const name = catalog.displayName;

  const installable = catalog.versions.filter((version) => version.available);
  const endOfLife = installable.filter((version) => version.eol);
  const visible = showEol ? installable : installable.filter((version) => !version.eol);
  const have = new Set(
    (installed ?? []).filter((entry) => entry.runtime === catalog.id).map((e) => e.version),
  );

  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <CardTitle>{t("runtimes.available.title")}</CardTitle>
            <CardDescription>{t("runtimes.available.description", { name })}</CardDescription>
          </div>
          {endOfLife.length > 0 && visible.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              aria-pressed={showEol}
              onClick={() => {
                setShowEol((current) => !current);
              }}
            >
              {showEol
                ? t("runtimes.available.hideEol")
                : t("runtimes.available.showEol", { count: endOfLife.length })}
            </Button>
          )}
        </div>
        {catalog.license && (
          <p className="text-xs text-muted-foreground">
            {t("runtimes.available.license", { name: catalog.license.name })}
            <span className="ml-1 font-mono select-all">{catalog.license.url}</span>
          </p>
        )}
      </CardHeader>
      <CardContent>
        {installable.length === 0 ? (
          <EmptyState
            icon={Boxes}
            title={t("runtimes.available.emptyTitle", { name })}
            description={t("runtimes.available.emptyDescription", { name })}
          />
        ) : visible.length === 0 ? (
          <div className="flex flex-col items-start gap-3">
            <p className="text-sm text-muted-foreground">
              {t("runtimes.available.allEndOfLife", { name })}
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setShowEol(true);
              }}
            >
              {t("runtimes.available.showEol", { count: endOfLife.length })}
            </Button>
          </div>
        ) : (
          <TableScroll label={t("runtimes.available.region", { name })}>
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead>{t("runtimes.available.columns.version")}</TableHead>
                  <TableHead>{t("runtimes.available.columns.notes")}</TableHead>
                  <TableHead>{t("runtimes.available.columns.released")}</TableHead>
                  <TableHead className="text-right">
                    {t("runtimes.available.columns.size")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {visible.map((version) => (
                  <TableRow key={version.version}>
                    <TableCell className="font-medium tabular-nums">{version.version}</TableCell>
                    <TableCell>
                      <VersionBadges version={version} installed={have.has(version.version)} />
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-muted-foreground tabular-nums">
                      {version.releasedAt ? formatIsoDate(version.releasedAt, i18n.language) : ""}
                    </TableCell>
                    <TableCell className="text-right whitespace-nowrap text-muted-foreground tabular-nums">
                      {version.size === null ? "" : formatBytes(version.size)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </TableScroll>
        )}
      </CardContent>
    </Card>
  );
}
