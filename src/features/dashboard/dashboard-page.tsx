import { Boxes, FolderKanban, Server, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { PageHeader } from "@/components/page-header";
import { StatCard } from "@/components/stat-card";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

type StepId = "runtimes" | "projects" | "services";

const STEPS: readonly { id: StepId; path: string; icon: LucideIcon }[] = [
  { id: "runtimes", path: "/runtimes", icon: Boxes },
  { id: "projects", path: "/projects", icon: FolderKanban },
  { id: "services", path: "/services", icon: Server },
];

// Counts are real zeros for now: nothing can be installed or added until the backend exists.
// They become live queries in the Runtimes / Services / Projects milestones.
export function DashboardPage() {
  const { t } = useTranslation();
  return (
    <>
      <PageHeader title={t("nav.dashboard")} description={t("pages.dashboard.description")} />

      <div className="grid grid-cols-[repeat(auto-fit,minmax(11rem,1fr))] gap-4">
        <StatCard
          icon={Server}
          label={t("dashboard.stats.services.label")}
          value={0}
          hint={t("dashboard.stats.services.hint")}
        />
        <StatCard
          icon={FolderKanban}
          label={t("dashboard.stats.projects.label")}
          value={0}
          hint={t("dashboard.stats.projects.hint")}
        />
        <StatCard
          icon={Boxes}
          label={t("dashboard.stats.runtimes.label")}
          value={0}
          hint={t("dashboard.stats.runtimes.hint")}
        />
      </div>

      <Card className="mt-6">
        <CardHeader>
          <CardTitle>{t("dashboard.start.title")}</CardTitle>
          <CardDescription>{t("dashboard.start.description")}</CardDescription>
        </CardHeader>
        <CardContent>
          <ol className="divide-y">
            {STEPS.map(({ id, path, icon: Icon }, index) => (
              <li key={id} className="flex items-center gap-4 py-4 first:pt-0 last:pb-0">
                <span
                  aria-hidden
                  className="flex size-8 shrink-0 items-center justify-center rounded-full bg-primary-soft text-sm font-semibold text-primary-soft-foreground tabular-nums"
                >
                  {index + 1}
                </span>
                <div className="min-w-0 flex-1">
                  <h3 className="font-medium">{t(`dashboard.start.${id}.title`)}</h3>
                  <p className="text-sm text-muted-foreground">
                    {t(`dashboard.start.${id}.description`)}
                  </p>
                </div>
                <Button asChild variant="outline" size="sm">
                  <Link to={path}>
                    <Icon aria-hidden />
                    {t(`dashboard.start.${id}.action`)}
                  </Link>
                </Button>
              </li>
            ))}
          </ol>
        </CardContent>
      </Card>
    </>
  );
}
