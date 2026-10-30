import { Ban, CircleAlert, CircleCheck, ListChecks, LoaderCircle, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { JobInfo } from "@/bindings/JobInfo";
import { ErrorState } from "@/components/error-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { useErrorCategory } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { percent } from "./jobs-state";
import { useCancelJob, useJobs } from "./use-jobs";

const KNOWN_PHASES = ["starting", "download", "verify", "extract"] as const;
type KnownPhase = (typeof KNOWN_PHASES)[number];

function isKnownPhase(phase: string): phase is KnownPhase {
  return (KNOWN_PHASES as readonly string[]).includes(phase);
}

const STATE_BADGE = {
  running: { variant: "info", icon: LoaderCircle },
  succeeded: { variant: "success", icon: CircleCheck },
  failed: { variant: "destructive", icon: CircleAlert },
  cancelled: { variant: "neutral", icon: Ban },
} as const;

function JobRow({ job, onCancel }: { job: JobInfo; onCancel: (id: string) => void }) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const badge = STATE_BADGE[job.state];
  const Icon = badge.icon;
  const running = job.state === "running";
  const value = percent(job.bytesDone, job.bytesTotal);

  const phase = isKnownPhase(job.phase) ? t(`jobs.phase.${job.phase}`) : job.phase;
  const bytes =
    job.bytesTotal !== null
      ? t("jobs.bytes.of", { done: formatBytes(job.bytesDone), total: formatBytes(job.bytesTotal) })
      : job.bytesDone > 0
        ? t("jobs.bytes.done", { done: formatBytes(job.bytesDone) })
        : null;

  return (
    <li className="flex flex-col gap-2.5 py-4 first:pt-0 last:pb-0">
      <div className="flex items-center gap-3">
        <p className="min-w-0 flex-1 truncate font-medium">{job.label}</p>
        <Badge variant={badge.variant}>
          <Icon
            className={running ? "size-3 animate-spin motion-reduce:animate-none" : "size-3"}
            aria-hidden
          />
          {t(`jobs.state.${job.state}`)}
        </Badge>
        {running && (
          <Button
            variant="ghost"
            size="sm"
            aria-label={t("jobs.cancelLabel", { label: job.label })}
            onClick={() => {
              onCancel(job.id);
            }}
          >
            <X aria-hidden />
            {t("jobs.cancel")}
          </Button>
        )}
      </div>

      {running && (
        <>
          <Progress value={value} label={t("jobs.progressLabel", { label: job.label })} />
          <div className="flex items-center justify-between gap-3 text-[13px] text-muted-foreground">
            <span className="truncate">
              {phase}
              {job.message ? ` · ${job.message}` : ""}
            </span>
            <span className="shrink-0 tabular-nums">
              {value !== null && `${String(value)}%`}
              {value !== null && bytes ? " · " : ""}
              {bytes}
            </span>
          </div>
        </>
      )}

      {job.state === "failed" && job.error && (
        <p className="text-[13px] text-muted-foreground">
          <span className="font-medium text-foreground">{categoryOf(job.error.code)}.</span>{" "}
          <span className="font-mono break-words">{job.error.message}</span>
        </p>
      )}
    </li>
  );
}

function JobsSkeleton() {
  const { t } = useTranslation();
  return (
    <div role="status" aria-label={t("jobs.loading")} className="space-y-3">
      <Skeleton className="h-5 w-56" />
      <Skeleton className="h-2 w-full" />
    </div>
  );
}

function NoJobs() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center py-8 text-center">
      <div className="flex size-10 items-center justify-center rounded-full bg-primary-soft text-primary-soft-foreground">
        <ListChecks className="size-5" aria-hidden />
      </div>
      <h3 className="mt-3 font-medium">{t("jobs.empty.title")}</h3>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">{t("jobs.empty.description")}</p>
    </div>
  );
}

/** Running and recent background tasks (downloads, installs), live from backend events. */
export function JobsCard() {
  const { t } = useTranslation();
  const query = useJobs();
  const cancel = useCancelJob();

  if (query.isError) {
    return (
      <div className="mt-6">
        <ErrorState
          error={query.error.error}
          title={t("jobs.loadError")}
          onRetry={() => {
            void query.refetch();
          }}
        />
      </div>
    );
  }

  return (
    <Card className="mt-6">
      <CardHeader>
        <CardTitle>{t("jobs.title")}</CardTitle>
        <CardDescription>{t("jobs.description")}</CardDescription>
      </CardHeader>
      <CardContent>
        {query.isPending ? (
          <JobsSkeleton />
        ) : query.data.length === 0 ? (
          <NoJobs />
        ) : (
          <ul className="divide-y">
            {query.data.map((job) => (
              <JobRow
                key={job.id}
                job={job}
                onCancel={(id) => {
                  cancel.mutate(id);
                }}
              />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
