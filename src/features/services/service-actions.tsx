import { LoaderCircle, Play, RotateCw, Square, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { JobInfo } from "@/bindings/JobInfo";
import type { ServiceRow } from "@/bindings/ServiceRow";
import { Button } from "@/components/ui/button";
import { serviceJobAction } from "./service-jobs";

interface ServiceActionsProps {
  row: ServiceRow;
  /** `MySQL 8.4`: what the buttons are called for a screen reader. */
  name: string;
  /** The newest start, stop or restart of this service. */
  job: JobInfo | undefined;
  /** A command was sent and has not been answered yet. */
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onCancel: (jobId: string) => void;
}

/**
 * What can be done with a service right now, from its state: Start when it is down (also after a failure), Restart
 * and Stop while it runs, only Stop while it is starting on its own (an automatic restart), nothing while it stops.
 * While its own start or restart job runs there is Cancel instead (a first start can take a minute).
 */
export function ServiceActions({
  row,
  name,
  job,
  busy,
  onStart,
  onStop,
  onRestart,
  onCancel,
}: ServiceActionsProps) {
  const { t } = useTranslation();
  const vars = { name };
  const jobAction = job?.state === "running" ? serviceJobAction(job) : null;

  if (job && jobAction !== null && jobAction !== "stop") {
    return (
      <Button
        variant="outline"
        size="sm"
        aria-label={t("services.actions.cancelLabel", vars)}
        title={t("services.actions.cancelLabel", vars)}
        onClick={() => {
          onCancel(job.id);
        }}
      >
        <X aria-hidden />
        <span className="max-lg:sr-only">{t("services.actions.cancel")}</span>
      </Button>
    );
  }

  if (jobAction === "stop" || row.state === "stopping") {
    return (
      <Button
        variant="outline"
        size="sm"
        disabled
        aria-label={t("services.actions.stoppingLabel", vars)}
        title={t("services.actions.stoppingLabel", vars)}
      >
        <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
        <span className="max-lg:sr-only">{t("services.actions.stopping")}</span>
      </Button>
    );
  }

  const stop = (
    <Button
      variant="outline"
      size="sm"
      disabled={busy}
      aria-label={t("services.actions.stopLabel", vars)}
      title={t("services.actions.stopLabel", vars)}
      onClick={onStop}
    >
      <Square aria-hidden />
      <span className="max-lg:sr-only">{t("services.actions.stop")}</span>
    </Button>
  );

  if (row.state === "running") {
    return (
      <>
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          aria-label={t("services.actions.restartLabel", vars)}
          title={t("services.actions.restartLabel", vars)}
          onClick={onRestart}
        >
          <RotateCw aria-hidden />
          <span className="max-lg:sr-only">{t("services.actions.restart")}</span>
        </Button>
        {stop}
      </>
    );
  }

  if (row.state === "starting") return stop;

  return (
    <Button
      variant="outline"
      size="sm"
      disabled={busy}
      aria-label={t("services.actions.startLabel", vars)}
      title={t("services.actions.startLabel", vars)}
      onClick={onStart}
    >
      <Play aria-hidden />
      <span className="max-lg:sr-only">{t("services.actions.start")}</span>
    </Button>
  );
}
