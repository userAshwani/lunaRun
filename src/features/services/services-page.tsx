import { Boxes, Server, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { Leftover } from "@/bindings/Leftover";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useCancelJob, useJobs } from "@/features/jobs/use-jobs";
import { serviceName } from "./service-format";
import { ServiceProblemView } from "./service-problem";
import { ServicesNote, ServicesTable } from "./services-table";
import { useServiceActions, useServiceJobEffects } from "./use-service-actions";
import { useLeftovers, useServices } from "./use-services";

function ServicesSkeleton() {
  const { t } = useTranslation();
  return (
    <Card>
      <CardContent>
        <div role="status" aria-label={t("services.loading")} className="space-y-3">
          {[0, 1, 2, 3].map((row) => (
            <Skeleton key={row} className="h-6 w-full" />
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

/** Services an earlier session left running. Read-only for now: adopting or stopping them is the next task. */
function LeftoversNotice({ leftovers }: { leftovers: readonly Leftover[] }) {
  const { t } = useTranslation();
  return (
    <div
      role="status"
      className="flex items-start gap-3 rounded-xl border border-warning/40 bg-warning/10 px-4 py-3"
    >
      <TriangleAlert className="mt-0.5 size-5 shrink-0 text-warning" aria-hidden />
      <div className="min-w-0">
        <p className="font-medium">{t("services.leftovers.title")}</p>
        <ul className="mt-1 text-sm tabular-nums">
          {leftovers.map((leftover) => (
            <li key={leftover.id}>
              {t("services.leftovers.item", { id: leftover.id, pid: leftover.pid })}
            </li>
          ))}
        </ul>
        <p className="mt-2 text-[13px] text-muted-foreground">
          {t("services.leftovers.description")}
        </p>
      </div>
    </div>
  );
}

export function ServicesPage() {
  const { t } = useTranslation();
  const services = useServices();
  const leftovers = useLeftovers();
  const jobs = useJobs();
  const cancel = useCancelJob();
  const actions = useServiceActions(jobs.data);
  const rows = services.data;
  const nameOf = (id: string) => {
    const found = rows?.find((row) => row.id === id);
    return found ? serviceName(found) : id;
  };
  useServiceJobEffects(nameOf);

  return (
    <>
      <PageHeader title={t("nav.services")} description={t("pages.services.description")} />

      {services.isError ? (
        <ErrorState
          error={services.error.error}
          title={t("services.loadError")}
          onRetry={() => {
            void services.refetch();
          }}
        />
      ) : !rows ? (
        <ServicesSkeleton />
      ) : (
        <div className="space-y-6">
          {leftovers.data && leftovers.data.length > 0 && (
            <LeftoversNotice leftovers={leftovers.data} />
          )}
          {actions.problems.length > 0 && (
            <div className="space-y-3">
              {actions.problems.map((problem) => (
                <ServiceProblemView
                  key={problem.id}
                  error={problem.error}
                  service={problem.service}
                  name={nameOf(problem.service)}
                  action={problem.action}
                  busy={actions.pending.has(problem.service)}
                  onUseAnotherPort={() => {
                    void actions.useAnotherPort(problem.service);
                  }}
                  onDismiss={() => {
                    actions.dismiss(problem);
                  }}
                />
              ))}
            </div>
          )}
          {rows.length === 0 ? (
            <EmptyState
              icon={Server}
              title={t("services.empty.title")}
              description={t("services.empty.description")}
              action={
                <Button asChild>
                  <Link to="/runtimes">
                    <Boxes aria-hidden />
                    {t("services.empty.action")}
                  </Link>
                </Button>
              }
            />
          ) : (
            <>
              <Card>
                <CardContent>
                  <ServicesTable
                    rows={rows}
                    jobs={actions.latest}
                    pending={actions.pending}
                    onStart={(id) => {
                      void actions.start(id);
                    }}
                    onStop={(id) => {
                      void actions.stop(id);
                    }}
                    onRestart={(id) => {
                      void actions.restart(id);
                    }}
                    onCancel={(jobId) => {
                      cancel.mutate(jobId);
                    }}
                  />
                </CardContent>
              </Card>
              <div className="space-y-2">
                {rows.some((row) => row.kind === "database") && (
                  <ServicesNote>{t("services.databaseNote")}</ServicesNote>
                )}
              </div>
            </>
          )}
        </div>
      )}
    </>
  );
}
