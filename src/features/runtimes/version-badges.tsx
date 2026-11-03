import { Check, CircleAlert, ShieldAlert, Users } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AvailableVersion } from "@/bindings/AvailableVersion";
import { Badge } from "@/components/ui/badge";
import { formatIsoDate } from "@/lib/format";

/** Everything worth knowing about one version at a glance. Each badge pairs an icon with text, never colour alone. */
export function VersionBadges({
  version,
  installed,
}: {
  version: AvailableVersion;
  installed: boolean;
}) {
  const { t, i18n } = useTranslation();
  const date = (iso: string) => formatIsoDate(iso, i18n.language);

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {installed && (
        <Badge variant="success">
          <Check className="size-3" aria-hidden />
          {t("runtimes.badge.installed")}
        </Badge>
      )}
      {version.channel !== "stable" && (
        <Badge variant="info">{t(`runtimes.badge.${version.channel}`)}</Badge>
      )}
      {version.eol ? (
        <Badge
          variant="destructive"
          title={
            version.eolDate
              ? t("runtimes.badge.endOfLifeSince", { date: date(version.eolDate) })
              : undefined
          }
        >
          <CircleAlert className="size-3" aria-hidden />
          {t("runtimes.badge.endOfLife")}
        </Badge>
      ) : version.securityOnly ? (
        <Badge variant="warning">
          <ShieldAlert className="size-3" aria-hidden />
          {version.eolDate
            ? t("runtimes.badge.securityOnlyUntil", { date: date(version.eolDate) })
            : t("runtimes.badge.securityOnly")}
        </Badge>
      ) : (
        version.eolDate && (
          <Badge>{t("runtimes.badge.supportedUntil", { date: date(version.eolDate) })}</Badge>
        )
      )}
      {version.community && (
        <Badge variant="info" title={t("runtimes.badge.communityHint")}>
          <Users className="size-3" aria-hidden />
          {t("runtimes.badge.community")}
        </Badge>
      )}
      {version.rosettaFallback && (
        <Badge title={t("runtimes.badge.rosettaHint")}>{t("runtimes.badge.rosetta")}</Badge>
      )}
    </div>
  );
}
