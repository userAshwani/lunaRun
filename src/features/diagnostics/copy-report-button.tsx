import { Check, ClipboardCopy, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AppInfo } from "@/bindings/AppInfo";
import { Button } from "@/components/ui/button";
import { buildReport } from "./report";

type CopyState = "idle" | "copied" | "failed";

export function CopyReportButton({ info }: { info: AppInfo }) {
  const { t } = useTranslation();
  const [state, setState] = useState<CopyState>("idle");

  // Return to the normal label after a moment.
  useEffect(() => {
    if (state === "idle") return;
    const timer = window.setTimeout(() => {
      setState("idle");
    }, 2000);
    return () => {
      window.clearTimeout(timer);
    };
  }, [state]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(buildReport(info));
      setState("copied");
    } catch {
      setState("failed");
    }
  };

  const Icon = state === "copied" ? Check : state === "failed" ? TriangleAlert : ClipboardCopy;
  const label =
    state === "copied"
      ? t("diagnostics.copy.done")
      : state === "failed"
        ? t("diagnostics.copy.failed")
        : t("diagnostics.copy.action");

  return (
    <Button
      variant="outline"
      onClick={() => {
        void copy();
      }}
    >
      <Icon aria-hidden />
      <span aria-live="polite">{label}</span>
    </Button>
  );
}
