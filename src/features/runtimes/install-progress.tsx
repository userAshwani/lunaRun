import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { JobInfo } from "@/bindings/JobInfo";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { useJobProgressText } from "@/features/jobs/job-progress";

interface InstallProgressProps {
  job: JobInfo;
  name: string;
  version: string;
  onCancel: (jobId: string) => void;
}

/** What an install shows in its row while it runs: phase, percentage, bytes, a bar and Cancel. */
export function InstallProgress({ job, name, version, onCancel }: InstallProgressProps) {
  const { t } = useTranslation();
  const { phase, bytes, value } = useJobProgressText()(job);
  const who = { name, version };

  return (
    <div className="flex items-center justify-end gap-2">
      <div className="w-48 space-y-1">
        <div className="flex items-baseline justify-between gap-2 text-xs text-muted-foreground">
          <span className="truncate">{phase}</span>
          {value !== null && <span className="shrink-0 tabular-nums">{String(value)}%</span>}
        </div>
        <Progress
          className="h-1.5"
          value={value}
          label={t("runtimes.actions.progressLabel", who)}
        />
        {bytes && (
          <p className="truncate text-[11px] text-muted-foreground tabular-nums">{bytes}</p>
        )}
      </div>
      <Button
        variant="ghost"
        size="sm"
        aria-label={t("runtimes.actions.cancelLabel", who)}
        onClick={() => {
          onCancel(job.id);
        }}
      >
        <X aria-hidden />
        {t("runtimes.actions.cancel")}
      </Button>
    </div>
  );
}
