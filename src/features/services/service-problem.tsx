import { CircleAlert, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { LunaError } from "@/bindings/LunaError";
import { Button } from "@/components/ui/button";
import { useErrorCategory } from "@/lib/errors";
import type { ServiceAction } from "./service-jobs";

interface ServiceProblemViewProps {
  error: LunaError;
  /** The row's id: Caddy's admin port has no other port to move to. */
  service: string;
  /** `MySQL 8.4`. */
  name: string;
  action: ServiceAction;
  /** A command for this service is on its way. */
  busy: boolean;
  onUseAnotherPort: () => void;
  onDismiss: () => void;
}

function numberAt(details: LunaError["details"], key: string): number | null {
  const value = details?.[key];
  return typeof value === "number" ? value : null;
}

function textAt(details: LunaError["details"], key: string): string | null {
  const value = details?.[key];
  return typeof value === "string" && value !== "" ? value : null;
}

function linesAt(details: LunaError["details"], key: string): string[] {
  const value = details?.[key];
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

/** Other local-server bundles that run their own MySQL or web server on the usual ports, told by where the program is. */
const PRODUCTS: readonly (readonly [RegExp, string])[] = [
  [/xampp/i, "XAMPP"],
  [/wamp/i, "WampServer"],
  [/laragon/i, "Laragon"],
  [/mamp/i, "MAMP"],
];

function productOf(path: string | null): string | null {
  if (!path) return null;
  return PRODUCTS.find(([pattern]) => pattern.test(path))?.[1] ?? null;
}

/**
 * What went wrong with a start, stop or restart, in plain words, with what to do about it: who holds a taken port and
 * a button to use another one, the end of a failed database setup, why Caddy refused its settings. The technical
 * message stays underneath for bug reports.
 */
export function ServiceProblemView({
  error,
  service,
  name,
  action,
  busy,
  onUseAnotherPort,
  onDismiss,
}: ServiceProblemViewProps) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const details = error.details;
  const who = { name };

  const headline = t(
    action === "stop"
      ? "services.problem.stopFailed"
      : action === "restart"
        ? "services.problem.restartFailed"
        : "services.problem.startFailed",
    who,
  );
  let advice: string | null = categoryOf(error.code);
  let hint: string | null = null;
  let output: string[] = [];
  let canMovePort = false;
  let installLink = false;

  switch (error.code) {
    case "PORT_IN_USE": {
      const port = numberAt(details, "port");
      const processName = textAt(details, "processName");
      const processPath = textAt(details, "processPath");
      const pid = numberAt(details, "pid");
      const reason = textAt(details, "reason");
      const holder = processName
        ? processPath
          ? t("services.problem.holder", { name: processName, path: processPath })
          : processName
        : pid !== null
          ? t("services.problem.holderPid", { pid })
          : null;
      if (port !== null) {
        advice = holder
          ? t("services.problem.portUsedBy", { port, holder })
          : reason
            ? t("services.problem.portUsedReason", { port, reason })
            : t("services.problem.portUsed", { port });
      }
      const product = productOf(processPath);
      if (service === "caddy") {
        hint = t("services.problem.caddyPort");
      } else {
        canMovePort = true;
        hint = product
          ? t("services.problem.productHint", { product })
          : t("services.problem.anotherPortHint");
      }
      break;
    }
    case "DATABASE_INIT_FAILED": {
      advice = t("services.problem.initFailed");
      output = linesAt(details, "tail");
      const log = textAt(details, "log");
      hint = log ? t("services.problem.fullOutput", { path: log }) : null;
      break;
    }
    case "CONFIG_INVALID":
      advice = t("services.problem.configInvalid", who);
      output = linesAt(details, "tail");
      break;
    case "RUNTIME_BROKEN":
      advice = t("services.problem.runtimeBroken");
      installLink = true;
      break;
    case "NOT_FOUND":
      advice = t("services.problem.notInstalled");
      installLink = true;
      break;
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
        {hint && <p className="text-muted-foreground">{hint}</p>}
        {output.length > 0 && (
          <pre
            tabIndex={0}
            aria-label={t("services.problem.outputLabel")}
            className="max-h-40 overflow-auto rounded-md bg-muted p-2 font-mono text-xs whitespace-pre-wrap text-foreground"
          >
            {output.join("\n")}
          </pre>
        )}
        <p className="font-mono text-xs break-words text-muted-foreground">{error.message}</p>
        {(canMovePort || installLink) && (
          <div className="flex flex-wrap gap-2 pt-1">
            {canMovePort && (
              <Button variant="outline" size="sm" disabled={busy} onClick={onUseAnotherPort}>
                {t("services.problem.useAnotherPort")}
              </Button>
            )}
            {installLink && (
              <Button variant="outline" size="sm" asChild>
                <Link to="/runtimes">{t("services.problem.openRuntimes")}</Link>
              </Button>
            )}
          </div>
        )}
      </div>
      <Button
        variant="ghost"
        size="icon"
        className="-m-1 size-7 shrink-0"
        aria-label={t("services.problem.dismiss")}
        onClick={onDismiss}
      >
        <X aria-hidden />
      </Button>
    </div>
  );
}
