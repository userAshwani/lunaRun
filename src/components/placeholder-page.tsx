import { useTranslation } from "react-i18next";
import { NAV_ITEMS, type PageId } from "@/app/routes";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";

/** Stand-in used by feature pages until they are built. */
export function PlaceholderPage({ page }: { page: PageId }) {
  const { t } = useTranslation();
  const item = NAV_ITEMS.find((entry) => entry.id === page);
  return (
    <>
      <PageHeader title={t(`nav.${page}`)} description={t(`pages.${page}.description`)} />
      {item && (
        <EmptyState
          icon={item.icon}
          title={t("pages.notImplemented.title")}
          description={t("pages.notImplemented.description")}
        />
      )}
    </>
  );
}
