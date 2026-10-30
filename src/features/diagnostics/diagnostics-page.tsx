import { useTranslation } from "react-i18next";
import type { AppInfo } from "@/bindings/AppInfo";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyReportButton } from "./copy-report-button";
import { useAppInfo } from "./use-app-info";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-1 gap-1 py-3 first:pt-0 last:pb-0 sm:grid-cols-[12rem_1fr] sm:gap-6">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 font-medium">{children}</dd>
    </div>
  );
}

function SystemDetails({ info }: { info: AppInfo }) {
  const { t } = useTranslation();
  return (
    <dl className="divide-y">
      <Row label={t("diagnostics.fields.version")}>
        <span className="tabular-nums">{info.appVersion}</span>
      </Row>
      <Row label={t("diagnostics.fields.os")}>{t(`diagnostics.os.${info.os}`)}</Row>
      <Row label={t("diagnostics.fields.arch")}>{t(`diagnostics.arch.${info.arch}`)}</Row>
      <Row label={t("diagnostics.fields.home")}>
        <span className="font-mono text-[13px] font-normal break-all">{info.lunaHome}</span>
        <Badge className="ml-2 align-middle" variant={info.lunaHomeFromEnv ? "info" : "neutral"}>
          {info.lunaHomeFromEnv ? t("diagnostics.home.fromEnv") : t("diagnostics.home.default")}
        </Badge>
      </Row>
    </dl>
  );
}

function SystemSkeleton() {
  const { t } = useTranslation();
  return (
    <div role="status" aria-label={t("diagnostics.loading")} className="space-y-4">
      {[0, 1, 2, 3].map((row) => (
        <div key={row} className="grid grid-cols-[12rem_1fr] gap-6">
          <Skeleton className="h-5 w-32" />
          <Skeleton className="h-5 w-64" />
        </div>
      ))}
    </div>
  );
}

export function DiagnosticsPage() {
  const { t } = useTranslation();
  const query = useAppInfo();

  return (
    <>
      <PageHeader
        title={t("nav.diagnostics")}
        description={t("pages.diagnostics.description")}
        actions={query.data && <CopyReportButton info={query.data} />}
      />

      {query.isError ? (
        <ErrorState
          error={query.error.error}
          title={t("diagnostics.loadError")}
          onRetry={() => {
            void query.refetch();
          }}
        />
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>{t("diagnostics.system.title")}</CardTitle>
            <CardDescription>{t("diagnostics.system.description")}</CardDescription>
          </CardHeader>
          <CardContent>
            {query.data ? <SystemDetails info={query.data} /> : <SystemSkeleton />}
          </CardContent>
        </Card>
      )}
    </>
  );
}
