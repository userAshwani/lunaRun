import { Server } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { LunaError } from "@/bindings/LunaError";
import type { ServiceRow } from "@/bindings/ServiceRow";
import { EmptyState } from "@/components/empty-state";
import { Button } from "@/components/ui/button";
import { formatPorts, serviceName } from "@/features/services/service-format";
import { ServiceStateBadge } from "@/features/services/service-state-badge";
import { Section, SectionError, SectionSkeleton } from "./section";

interface RunningServicesProps {
  /** The services doing something (`activeServices`). */
  active: readonly ServiceRow[];
  loading: boolean;
  error: LunaError | null;
  onRetry: () => void;
}

/** What runs now, from `services_list`, kept live by the `service:state` events (D-031). */
export function RunningServices({ active, loading, error, onRetry }: RunningServicesProps) {
  const { t } = useTranslation();
  let body;
  if (loading) {
    body = <SectionSkeleton label={t("dashboard.services.loading")} rows={2} />;
  } else if (error) {
    body = <SectionError error={error} title={t("dashboard.services.failed")} onRetry={onRetry} />;
  } else if (active.length === 0) {
    body = (
      <EmptyState
        compact
        icon={Server}
        title={t("dashboard.services.emptyTitle")}
        description={t("dashboard.services.emptyDescription")}
      />
    );
  } else {
    body = (
      <ul className="divide-y px-6 pb-2">
        {active.map((row) => (
          <li key={row.id} className="flex items-center gap-3 py-3">
            <div className="min-w-0 flex-1">
              <p className="truncate font-medium">{serviceName(row)}</p>
              {row.ports.length > 0 && (
                <p className="text-[13px] text-muted-foreground tabular-nums">
                  {t("dashboard.services.ports", { ports: formatPorts(row.ports) })}
                </p>
              )}
            </div>
            <ServiceStateBadge state={row.state} />
          </li>
        ))}
      </ul>
    );
  }
  return (
    <Section
      title={t("dashboard.services.title")}
      description={t("dashboard.services.description")}
      action={
        <Button asChild variant="ghost" size="sm" className="text-primary-soft-foreground">
          <Link to="/services">{t("dashboard.services.manage")}</Link>
        </Button>
      }
    >
      {body}
    </Section>
  );
}
