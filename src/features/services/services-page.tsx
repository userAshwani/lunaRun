import { Boxes, LoaderCircle, Server, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { Leftover } from "@/bindings/Leftover";
import type { LunaError } from "@/bindings/LunaError";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useErrorCategory } from "@/lib/errors";
import { useCancelJob, useJobs } from "@/features/jobs/use-jobs";
import { serviceName } from "./service-format";
import { ServiceLogPanel } from "./service-log-panel";
import { ServiceProblemView } from "./service-problem";
import { ServicesNote, ServicesTable } from "./services-table";
import { useLeftoverActions } from "./use-leftover-actions";
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

interface LeftoversNoticeProps {
  leftovers: readonly Leftover[];
  /** `MySQL 8.4` for `mysql@8.4`, or the id itself when the service is not in the list. */
  nameOf: (id: string) => string;
  pending: ReadonlyMap<string, "adopt" | "stop">;
  errors: ReadonlyMap<string, LunaError>;
  onAdopt: (id: string) => void;
  /** Ask first: stopping ends a program the user may be using. */
  onStop: (id: string) => void;
}

/** Services an earlier session left running, each with the two things that can be done about it. */
function LeftoversNotice({
  leftovers,
  nameOf,
  pending,
  errors,
  onAdopt,
  onStop,
}: LeftoversNoticeProps) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  return (
    <div
      role="status"
      className="flex items-start gap-3 rounded-xl border border-warning/40 bg-warning/10 px-4 py-3"
    >
      <TriangleAlert className="mt-0.5 size-5 shrink-0 text-warning" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{t("services.leftovers.title")}</p>
        <p className="mt-1 text-[13px] text-muted-foreground">
          {t("services.leftovers.description")}
        </p>
        <ul className="mt-3 space-y-3">
          {leftovers.map((leftover) => {
            const doing = pending.get(leftover.id);
            const failure = errors.get(leftover.id);
            const name = nameOf(leftover.id);
            return (
              <li key={leftover.id}>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-sm tabular-nums">
                    {t("services.leftovers.item", { name, pid: leftover.pid })}
                  </span>
                  <span className="flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={doing !== undefined}
                      aria-label={t("services.leftovers.adoptLabel", { name })}
                      onClick={() => {
                        onAdopt(leftover.id);
                      }}
                    >
                      {doing === "adopt" && (
                        <LoaderCircle
                          className="animate-spin motion-reduce:animate-none"
                          aria-hidden
                        />
                      )}
                      {t("services.leftovers.adopt")}
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={doing !== undefined}
                      aria-label={t("services.leftovers.stopLabel", { name })}
                      onClick={() => {
                        onStop(leftover.id);
                      }}
                    >
                      {doing === "stop" && (
                        <LoaderCircle
                          className="animate-spin motion-reduce:animate-none"
                          aria-hidden
                        />
                      )}
                      {t("services.leftovers.stop")}
                    </Button>
                  </span>
                </div>
                {failure && (
                  <p role="alert" className="mt-1 text-[13px] text-destructive">
                    {t("services.leftovers.failed", { name, reason: categoryOf(failure.code) })}{" "}
                    <span className="font-mono text-xs break-words">{failure.message}</span>
                  </p>
                )}
              </li>
            );
          })}
        </ul>
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
  const leftoverActions = useLeftoverActions(nameOf);
  const [stopping, setStopping] = useState<string | null>(null);
  const [logsFor, setLogsFor] = useState<string | null>(null);

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
            <LeftoversNotice
              leftovers={leftovers.data}
              nameOf={nameOf}
              pending={leftoverActions.pending}
              errors={leftoverActions.errors}
              onAdopt={(id) => {
                void leftoverActions.adopt(id);
              }}
              onStop={setStopping}
            />
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
                    logsFor={logsFor}
                    onLogs={(id) => {
                      setLogsFor((current) => (current === id ? null : id));
                    }}
                  />
                </CardContent>
              </Card>
              {logsFor !== null && rows.some((row) => row.id === logsFor) && (
                <ServiceLogPanel
                  key={logsFor}
                  rowId={logsFor}
                  name={nameOf(logsFor)}
                  onClose={() => {
                    setLogsFor(null);
                  }}
                />
              )}
              <div className="space-y-2">
                {rows.some((row) => row.kind === "database") && (
                  <ServicesNote>{t("services.databaseNote")}</ServicesNote>
                )}
              </div>
            </>
          )}
        </div>
      )}
      {stopping !== null && (
        <ConfirmDialog
          title={t("services.leftovers.confirmTitle", { name: nameOf(stopping) })}
          description={t("services.leftovers.confirmDescription")}
          confirmLabel={t("services.leftovers.confirmStop")}
          cancelLabel={t("services.leftovers.confirmKeep")}
          destructive
          onConfirm={() => {
            void leftoverActions.stop(stopping);
            setStopping(null);
          }}
          onCancel={() => {
            setStopping(null);
          }}
        />
      )}
    </>
  );
}
