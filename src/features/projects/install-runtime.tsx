import { Download, LoaderCircle } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { LunaError } from "@/bindings/LunaError";
import { Button } from "@/components/ui/button";
import { useCancelJob, useJobs } from "@/features/jobs/use-jobs";
import { InstallProgress } from "@/features/runtimes/install-progress";
import { RuntimeProblem } from "@/features/runtimes/runtime-problem";
import { jobKey, latestRuntimeJobs } from "@/features/runtimes/runtime-jobs";
import { useInstallRuntime } from "@/features/runtimes/use-runtimes";
import { IpcError, toLunaError } from "@/lib/ipc";
import { runtimeName } from "./project-format";

interface InstallRuntimeProps {
  runtime: string;
  version: string;
  projectName: string;
}

/**
 * Installs the version a project needs, from the project page: the button, then the same progress and Cancel the
 * Runtimes page shows (it is the same background job), then what went wrong with what to do about it. Once the
 * install is over the project is scanned again, which is what makes the row turn ready.
 */
export function InstallRuntime({ runtime, version, projectName }: InstallRuntimeProps) {
  const { t } = useTranslation();
  const jobs = useJobs();
  const cancel = useCancelJob();
  const install = useInstallRuntime();
  const [startError, setStartError] = useState<LunaError | null>(null);
  const name = runtimeName(runtime);
  const job = latestRuntimeJobs(jobs.data ?? []).get(jobKey(runtime, version));

  const start = () => {
    setStartError(null);
    install.mutate(
      { runtime, version },
      {
        onError: (thrown) => {
          setStartError(thrown instanceof IpcError ? thrown.error : toLunaError(thrown));
        },
      },
    );
  };

  if (job?.state === "running") {
    return (
      <InstallProgress
        job={job}
        name={name}
        version={version}
        onCancel={(id) => {
          cancel.mutate(id);
        }}
      />
    );
  }
  if (job?.state === "succeeded") {
    return (
      <p role="status" className="flex items-center gap-2 text-[13px] text-muted-foreground">
        <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" aria-hidden />
        {t("projects.install.refreshing")}
      </p>
    );
  }

  const failure = startError ?? (job?.state === "failed" ? job.error : null);
  return (
    <div className="flex flex-col items-end gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={install.isPending}
        aria-label={`${failure ? `${t("projects.install.retry")}: ` : ""}${t("projects.install.buttonLabel", { name, version, project: projectName })}`}
        onClick={start}
      >
        {install.isPending ? (
          <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
        ) : (
          <Download aria-hidden />
        )}
        {failure ? t("projects.install.retry") : t("projects.install.button", { name, version })}
      </Button>
      {failure && (
        <div className="w-full max-w-md text-left">
          <RuntimeProblem error={failure} name={name} version={version} action="install" />
        </div>
      )}
    </div>
  );
}
