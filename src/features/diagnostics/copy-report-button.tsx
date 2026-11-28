import { useTranslation } from "react-i18next";
import { CopyButton } from "@/components/copy-button";

/** Copy the report the backend built and scrubbed (D-046): no user name, no secrets, no project names. */
export function CopyReportButton({ text }: { text: string }) {
  const { t } = useTranslation();
  return (
    <CopyButton
      text={text}
      label={t("diagnostics.copy.action")}
      doneLabel={t("diagnostics.copy.done")}
      failedLabel={t("diagnostics.copy.failed")}
    />
  );
}
