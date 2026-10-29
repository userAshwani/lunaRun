import { useTranslation } from "react-i18next";
import type { PageId } from "@/app/routes";

/** Stand-in used by feature pages until they are built. */
export function PlaceholderPage({ page }: { page: PageId }) {
  const { t } = useTranslation();
  return (
    <section>
      <h1 className="text-2xl font-semibold">{t(`nav.${page}`)}</h1>
      <p className="mt-1 text-muted-foreground">{t(`pages.${page}.description`)}</p>
      <p className="mt-6 rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
        {t("pages.notImplemented")}
      </p>
    </section>
  );
}
