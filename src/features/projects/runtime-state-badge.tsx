import { CircleAlert, CircleCheck, Download } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { runtimeName, type RuntimeState } from "./project-format";

/**
 * One runtime of one project as a badge: the version that will be used, or what has to happen first. Every state has an
 * icon and words, so none is told apart by colour alone. A runtime the project does not use is plain muted text.
 */
export function RuntimeStateBadge({ runtime, state }: { runtime: string; state: RuntimeState }) {
  const { t } = useTranslation();
  const name = runtimeName(runtime);
  switch (state.kind) {
    case "ready":
      return (
        <Badge variant="success">
          <CircleCheck className="size-3" aria-hidden />
          {t("projects.runtime.ready", { name, version: state.version })}
        </Badge>
      );
    case "install":
      return (
        <Badge variant="warning">
          <Download className="size-3" aria-hidden />
          {t("projects.runtime.install", { name, version: state.version })}
          <span className="sr-only">{`. ${t("projects.status.install")}`}</span>
        </Badge>
      );
    case "none":
      return (
        <Badge variant="destructive">
          <CircleAlert className="size-3" aria-hidden />
          {t("projects.runtime.none")}
        </Badge>
      );
    case "notRequired":
      return (
        <span className="text-[13px] text-muted-foreground">
          {t("projects.runtime.notRequired")}
        </span>
      );
  }
}
