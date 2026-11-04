import { useTranslation } from "react-i18next";
import type { AppInfo } from "@/bindings/AppInfo";
import { CopyButton } from "@/components/copy-button";
import { buildReport } from "./report";

export function CopyReportButton({ info }: { info: AppInfo }) {
  const { t } = useTranslation();
  return (
    <CopyButton
      text={buildReport(info)}
      label={t("diagnostics.copy.action")}
      doneLabel={t("diagnostics.copy.done")}
      failedLabel={t("diagnostics.copy.failed")}
    />
  );
}
