import { CircleAlert, CircleCheck } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { LunaError } from "@/bindings/LunaError";
import { Button } from "@/components/ui/button";
import { serviceName } from "@/features/services/service-format";
import { jobPage, type Problem } from "./dashboard-model";
import { Section, SectionError, SectionSkeleton } from "./section";

interface ProblemsCardProps {
  problems: readonly Problem[];
  loading: boolean;
  /** The services or the tasks could not be read: then "No problems" would not be true. */
  error: LunaError | null;
  onRetry: () => void;
}

/**
 * What needs attention now (D-046's facts, as they are at this moment: D-047). Each problem says what failed, the
 * technical message, and links to the page where it is fixed. Diagnostics has the whole picture.
 */
export function ProblemsCard({ problems, loading, error, onRetry }: ProblemsCardProps) {
  const { t } = useTranslation();
  let body;
  if (loading) {
    body = <SectionSkeleton label={t("dashboard.problems.loading")} rows={1} />;
  } else if (error) {
    body = <SectionError error={error} title={t("dashboard.problems.failed")} onRetry={onRetry} />;
  } else if (problems.length === 0) {
    body = (
      <div className="flex items-start gap-3 px-6 pt-3 pb-5">
        <CircleCheck className="mt-0.5 size-5 shrink-0 text-success" aria-hidden />
        <div>
          <p className="font-medium">{t("dashboard.problems.noneTitle")}</p>
          <p className="text-[13px] text-muted-foreground">
            {t("dashboard.problems.noneDescription")}
          </p>
        </div>
      </div>
    );
  } else {
    body = (
      <ul className="divide-y px-6 pb-2">
        {problems.map((problem) => (
          <ProblemItem key={problemKey(problem)} problem={problem} />
        ))}
      </ul>
    );
  }
  return (
    <Section
      title={t("dashboard.problems.title")}
      description={t("dashboard.problems.description")}
      action={
        <Button asChild variant="ghost" size="sm" className="text-primary-soft-foreground">
          <Link to="/diagnostics">{t("dashboard.problems.diagnostics")}</Link>
        </Button>
      }
    >
      {body}
    </Section>
  );
}

function problemKey(problem: Problem): string {
  if (problem.kind === "service") return `service:${problem.row.id}`;
  if (problem.kind === "job") return `job:${problem.job.id}`;
  return "sites";
}

/** One problem: what failed, the technical message, and the page where it is fixed. */
function ProblemItem({ problem }: { problem: Problem }) {
  const { t } = useTranslation();
  let title: string;
  let message: string | null;
  let to: string;
  let fix: string;
  if (problem.kind === "service") {
    const name = serviceName(problem.row);
    if (problem.gaveUp) title = t("dashboard.problems.gaveUp", { name });
    else if (problem.crashed) title = t("dashboard.problems.crashed", { name });
    else title = t("dashboard.problems.error", { name });
    message = problem.error?.message ?? null;
    to = "/services";
    fix = t("dashboard.problems.fixService");
  } else if (problem.kind === "job") {
    // The backend's label says what was attempted ("Install PHP 8.3.14"); its error says why it failed.
    title = t("dashboard.problems.job", { label: problem.job.label });
    message = problem.job.error?.message ?? null;
    to = jobPage(problem.job);
    fix = t("dashboard.problems.fixJob");
  } else {
    title = t("dashboard.problems.sites");
    message = problem.error.message;
    to = "/projects";
    fix = t("dashboard.problems.fixSites");
  }
  return (
    <li className="flex items-start gap-3 py-3">
      <CircleAlert className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{title}</p>
        {message && (
          <p className="mt-1 line-clamp-3 font-mono text-xs break-words text-muted-foreground">
            {message}
          </p>
        )}
      </div>
      <Button asChild variant="outline" size="sm" className="shrink-0">
        <Link to={to}>{fix}</Link>
      </Button>
    </li>
  );
}
