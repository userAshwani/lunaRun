import { CircleAlert, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { LunaError } from "@/bindings/LunaError";
import { CopyButton } from "@/components/copy-button";
import { Button } from "@/components/ui/button";
import { useErrorCategory } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import type { JobKind } from "./runtime-jobs";

interface RuntimeProblemProps {
  error: LunaError;
  /** Display name of the runtime ("PHP"). */
  name: string;
  version: string;
  action: JobKind;
  onDismiss?: () => void;
}

function numberAt(details: LunaError["details"], key: string): number | null {
  const value = details?.[key];
  return typeof value === "number" ? value : null;
}

function textAt(details: LunaError["details"], key: string): string | null {
  const value = details?.[key];
  return typeof value === "string" && value !== "" ? value : null;
}

function listAt(details: LunaError["details"], key: string): string[] {
  const value = details?.[key];
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

/**
 * What went wrong with an install or uninstall, in plain words, with what to do about it. The technical
 * message stays underneath for bug reports. A missing Windows component gets its guidance and address.
 */
export function RuntimeProblem({ error, name, version, action, onDismiss }: RuntimeProblemProps) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const details = error.details;
  const who = { name, version };

  let headline: string = t(
    action === "install" ? "runtimes.problem.installFailed" : "runtimes.problem.uninstallFailed",
    who,
  );
  let advice: string | null = categoryOf(error.code);
  let guidance: {
    address: string | null;
    found: string | null;
    minimum: string | null;
    component: string;
  } | null = null;

  switch (error.code) {
    case "INSUFFICIENT_DISK": {
      const needed = numberAt(details, "needed");
      const free = numberAt(details, "free");
      if (needed !== null && free !== null) {
        advice = t("runtimes.problem.disk", {
          ...who,
          needed: formatBytes(needed),
          free: formatBytes(free),
        });
      }
      break;
    }
    case "ALREADY_INSTALLED":
      advice = t("runtimes.problem.alreadyInstalled", who);
      break;
    case "RUNTIME_IN_USE": {
      const services = listAt(details, "services");
      advice =
        services.length > 0
          ? t("runtimes.problem.inUseServices", { ...who, services: services.join(", ") })
          : t("runtimes.problem.inUse", who);
      break;
    }
    case "PROJECTS_BOUND":
      advice = t("runtimes.problem.projectsBound", {
        ...who,
        projects: listAt(details, "projects").join(", "),
      });
      break;
    case "BUSY":
      advice = t("runtimes.problem.busy", who);
      break;
    case "CHECKSUM_MISMATCH":
      advice = t("runtimes.problem.checksum", who);
      break;
    case "NETWORK":
      advice = t("runtimes.problem.network", who);
      break;
    case "NO_ARTIFACT_FOR_PLATFORM":
      advice = t("runtimes.problem.noBuild", who);
      break;
    case "PREREQUISITE_MISSING": {
      const component = textAt(details, "name") ?? categoryOf(error.code);
      headline = t("runtimes.problem.prerequisite", { ...who, component });
      advice = null;
      guidance = {
        component,
        address: textAt(details, "downloadUrl"),
        found: textAt(details, "found"),
        minimum: textAt(details, "minimum"),
      };
      break;
    }
    default:
      break;
  }

  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1 space-y-1.5 text-sm">
        <p className="font-medium">{headline}</p>
        {advice && <p className="text-muted-foreground">{advice}</p>}
        {guidance && (
          <div className="space-y-2 text-muted-foreground">
            <p>
              {guidance.found && guidance.minimum
                ? t("runtimes.problem.prerequisiteFound", {
                    found: guidance.found,
                    minimum: guidance.minimum,
                  })
                : t("runtimes.problem.prerequisiteMissing")}
            </p>
            {guidance.address && (
              <>
                <p>{t("runtimes.problem.prerequisiteDownload")}</p>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-xs break-all text-foreground select-all">
                    {guidance.address}
                  </span>
                  <CopyButton
                    text={guidance.address}
                    size="sm"
                    label={t("runtimes.problem.copyAddress")}
                    doneLabel={t("runtimes.problem.copiedAddress")}
                    failedLabel={t("runtimes.problem.copyFailed")}
                  />
                </div>
              </>
            )}
          </div>
        )}
        <p className="font-mono text-xs break-words text-muted-foreground">{error.message}</p>
      </div>
      {onDismiss && (
        <Button
          variant="ghost"
          size="icon"
          className="-m-1 size-7 shrink-0"
          aria-label={t("runtimes.problem.dismiss")}
          onClick={onDismiss}
        >
          <X aria-hidden />
        </Button>
      )}
    </div>
  );
}
