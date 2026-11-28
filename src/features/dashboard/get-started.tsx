import { Boxes, Check, FolderKanban, Play, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

type StepId = "runtimes" | "projects" | "services";

const STEPS: readonly { id: StepId; path: string; icon: LucideIcon }[] = [
  { id: "runtimes", path: "/runtimes", icon: Boxes },
  { id: "projects", path: "/projects", icon: FolderKanban },
  { id: "services", path: "/projects", icon: Play },
];

/**
 * The three steps to a first site, each ticked when it is done. The page shows it only until something is installed
 * and a project is added (D-047): after that it is in the way.
 */
export function GetStarted({ done }: { done: Readonly<Record<StepId, boolean>> }) {
  const { t } = useTranslation();
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("dashboard.start.title")}</CardTitle>
        <CardDescription>{t("dashboard.start.description")}</CardDescription>
      </CardHeader>
      <CardContent>
        <ol className="divide-y">
          {STEPS.map(({ id, path, icon: Icon }, index) => (
            <li key={id} className="flex items-center gap-4 py-4 first:pt-0 last:pb-0">
              {done[id] ? (
                <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-success/10 text-success-soft-foreground">
                  <Check className="size-4" aria-hidden />
                  <span className="sr-only">{t("dashboard.start.done")}</span>
                </span>
              ) : (
                <span
                  aria-hidden
                  className="flex size-8 shrink-0 items-center justify-center rounded-full bg-primary-soft text-sm font-semibold text-primary-soft-foreground tabular-nums"
                >
                  {index + 1}
                </span>
              )}
              <div className="min-w-0 flex-1">
                <h3 className={done[id] ? "font-medium text-muted-foreground" : "font-medium"}>
                  {t(`dashboard.start.${id}.title`)}
                </h3>
                <p className="text-sm text-muted-foreground">
                  {t(`dashboard.start.${id}.description`)}
                </p>
              </div>
              {!done[id] && (
                <Button asChild variant="outline" size="sm">
                  <Link to={path}>
                    <Icon aria-hidden />
                    {t(`dashboard.start.${id}.action`)}
                  </Link>
                </Button>
              )}
            </li>
          ))}
        </ol>
      </CardContent>
    </Card>
  );
}
