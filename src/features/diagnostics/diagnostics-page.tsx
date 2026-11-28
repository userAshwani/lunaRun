import { RotateCw } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { DiagnosticsReport } from "@/bindings/DiagnosticsReport";
import type { PortCheck } from "@/bindings/PortCheck";
import type { ServiceState } from "@/bindings/ServiceState";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
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
} from "@/components/ui/table";
import { JobsCard } from "@/features/jobs/jobs-card";
import { formatBytes } from "@/lib/format";
import { CopyReportButton } from "./copy-report-button";
import { useDiagnostics } from "./use-app-info";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-1 gap-1 py-3 first:pt-0 last:pb-0 sm:grid-cols-[12rem_1fr] sm:gap-6">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 font-medium">{children}</dd>
    </div>
  );
}

function SystemDetails({ report }: { report: DiagnosticsReport }) {
  const { t } = useTranslation();
  const info = report.app;
  const vc = report.vcRedist;
  return (
    <dl className="divide-y">
      <Row label={t("diagnostics.fields.version")}>
        <span className="tabular-nums">{info.appVersion}</span>
      </Row>
      <Row label={t("diagnostics.fields.os")}>{t(`diagnostics.os.${info.os}`)}</Row>
      <Row label={t("diagnostics.fields.arch")}>{t(`diagnostics.arch.${info.arch}`)}</Row>
      <Row label={t("diagnostics.fields.home")}>
        <span className="font-mono text-[13px] font-normal break-all">{info.lunaHome}</span>
        <Badge
          className="ml-2 align-middle"
          variant={report.homeSource === "default" ? "neutral" : "info"}
        >
          {report.homeSource === "env"
            ? t("diagnostics.home.fromEnv")
            : report.homeSource === "chosen"
              ? t("diagnostics.more.homeChosen")
              : t("diagnostics.home.default")}
        </Badge>
      </Row>
      <Row label={t("diagnostics.more.disk")}>
        <span className="tabular-nums">
          {report.freeBytes === null
            ? t("diagnostics.more.diskUnknown")
            : formatBytes(report.freeBytes)}
        </span>
      </Row>
      {vc !== "notNeeded" && (
        <Row label={t("diagnostics.more.vc")}>
          <Badge variant={vc === "ok" ? "success" : vc === "unknown" ? "neutral" : "warning"}>
            {t(`diagnostics.more.vcState.${vc}`)}
          </Badge>
        </Row>
      )}
      <Row label={t("diagnostics.more.projects")}>
        <span className="tabular-nums">{report.projects}</span>
      </Row>
      <Row label={t("diagnostics.more.installed")}>
        {report.installed.length === 0
          ? t("diagnostics.more.nothingInstalled")
          : report.installed.join(", ")}
      </Row>
      <Row label={t("diagnostics.more.settings")}>
        {report.changedSettings.length === 0
          ? t("diagnostics.more.noSettings")
          : report.changedSettings.join("; ")}
      </Row>
    </dl>
  );
}

function PortState({ check }: { check: PortCheck }) {
  const { t } = useTranslation();
  if (!check.held) return <Badge variant="success">{t("diagnostics.ports.free")}</Badge>;
  if (check.byLunarun) return <Badge variant="info">{t("diagnostics.ports.ours")}</Badge>;
  return (
    <Badge variant="warning">
      {check.owner
        ? t("diagnostics.ports.taken", { owner: check.owner })
        : t("diagnostics.ports.takenUnknown")}
    </Badge>
  );
}

function PortsCard({ report }: { report: DiagnosticsReport }) {
  const { t } = useTranslation();
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("diagnostics.ports.title")}</CardTitle>
        <CardDescription>{t("diagnostics.ports.description")}</CardDescription>
      </CardHeader>
      <CardContent>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-24">{t("diagnostics.ports.port")}</TableHead>
              <TableHead>{t("diagnostics.ports.usedFor")}</TableHead>
              <TableHead>{t("diagnostics.ports.state")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {report.ports.map((check) => (
              <TableRow key={check.port}>
                <TableCell className="font-medium tabular-nums">{check.port}</TableCell>
                <TableCell className="text-muted-foreground">
                  {check.assignedTo ?? t("diagnostics.ports.common")}
                </TableCell>
                <TableCell>
                  <PortState check={check} />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

const STATE_TONE: Record<ServiceState, "success" | "warning" | "destructive" | "neutral" | "info"> =
  {
    running: "success",
    starting: "info",
    stopping: "info",
    stopped: "neutral",
    crashed: "destructive",
    error: "destructive",
  };

function ServicesCard({ report }: { report: DiagnosticsReport }) {
  const { t } = useTranslation();
  const empty = report.services.length === 0 && report.failedJobs.length === 0;
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("diagnostics.services.title")}</CardTitle>
        <CardDescription>{t("diagnostics.services.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {empty && <p className="text-sm text-muted-foreground">{t("diagnostics.services.none")}</p>}
        {report.services.length > 0 && (
          <ul className="divide-y">
            {report.services.map((service) => (
              <li key={service.id} className="space-y-1 py-2 first:pt-0 last:pb-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{service.id}</span>
                  <Badge variant={STATE_TONE[service.state]}>
                    {t(`services.state.${service.state}`)}
                  </Badge>
                </div>
                {service.error && (
                  <p className="font-mono text-xs break-words text-muted-foreground">
                    {service.error}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
        {report.failedJobs.length > 0 && (
          <div className="space-y-2">
            <p className="text-sm font-medium">{t("diagnostics.services.failedJobs")}</p>
            <ul className="space-y-2">
              {report.failedJobs.map((job, index) => (
                <li key={`${job.label}-${String(index)}`}>
                  <p className="text-sm">{job.label}</p>
                  <p className="font-mono text-xs break-words text-muted-foreground">{job.error}</p>
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function RegistryCard({ report }: { report: DiagnosticsReport }) {
  const { t } = useTranslation();
  const r = report.registry;
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("diagnostics.registry.title")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-sm">
        <p>
          {t(`diagnostics.registry.source.${r.source}`)}
          {r.ageSeconds !== null &&
            ` · ${t("diagnostics.registry.age", { minutes: Math.floor(r.ageSeconds / 60) })}`}
        </p>
        {r.customHost && (
          <p className="text-muted-foreground">
            {t("diagnostics.registry.custom", { host: r.customHost })}
          </p>
        )}
        {r.problem && (
          <p className="text-warning-soft-foreground">
            {t("diagnostics.registry.problem", { problem: r.problem })}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function PageSkeleton() {
  const { t } = useTranslation();
  return (
    <Card>
      <CardContent className="pt-6">
        <div role="status" aria-label={t("diagnostics.loading")} className="space-y-4">
          {[0, 1, 2, 3].map((row) => (
            <div key={row} className="grid grid-cols-[12rem_1fr] gap-6">
              <Skeleton className="h-5 w-32" />
              <Skeleton className="h-5 w-64" />
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

/**
 * Diagnostics (FR-AP-04, D-046): the system, the data folder and its free space, Visual C++ on Windows, the ports and who
 * holds them, the services and what failed, the version list; and a report to copy, built and scrubbed by the backend.
 */
export function DiagnosticsPage() {
  const { t } = useTranslation();
  const query = useDiagnostics();
  const view = query.data;

  return (
    <>
      <PageHeader
        title={t("nav.diagnostics")}
        description={t("pages.diagnostics.description")}
        actions={
          view && (
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                disabled={query.isFetching}
                onClick={() => {
                  void query.refetch();
                }}
              >
                <RotateCw
                  className={
                    query.isFetching ? "animate-spin motion-reduce:animate-none" : undefined
                  }
                  aria-hidden
                />
                {t("diagnostics.checkAgain")}
              </Button>
              <CopyReportButton text={view.text} />
            </div>
          )
        }
      />

      {query.isError ? (
        <ErrorState
          error={query.error.error}
          title={t("diagnostics.loadError")}
          onRetry={() => {
            void query.refetch();
          }}
        />
      ) : !view ? (
        <PageSkeleton />
      ) : (
        <div className="space-y-6">
          <p className="text-[13px] text-muted-foreground">{t("diagnostics.reportNote")}</p>
          <Card>
            <CardHeader>
              <CardTitle>{t("diagnostics.system.title")}</CardTitle>
              <CardDescription>{t("diagnostics.system.description")}</CardDescription>
            </CardHeader>
            <CardContent>
              <SystemDetails report={view.report} />
            </CardContent>
          </Card>
          <PortsCard report={view.report} />
          <ServicesCard report={view.report} />
          <RegistryCard report={view.report} />
        </div>
      )}

      <JobsCard />
    </>
  );
}
