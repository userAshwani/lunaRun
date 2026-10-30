import { useTranslation } from "react-i18next";
import type { ErrorCode } from "@/bindings/ErrorCode";

/**
 * Returns a function that turns an error code into its translated category, for example
 * `CHECKSUM_MISMATCH` into "Download failed verification". A code this build does not know (sent by a
 * newer backend) falls back to the generic "Unexpected error" instead of showing a raw key.
 */
export function useErrorCategory(): (code: string) => string {
  const { t, i18n } = useTranslation();
  return (code) =>
    i18n.exists(`errors.${code}`) ? t(`errors.${code as ErrorCode}`) : t("errors.INTERNAL");
}
