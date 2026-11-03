import { CircleCheck, CloudOff, Info, LoaderCircle, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { RegistryStatus } from "@/bindings/RegistryStatus";

/** "3 hours", "2 days": the coarsest unit that still reads naturally. */
function useAge() {
  const { t } = useTranslation();
  return (seconds: number): string => {
    const minutes = Math.floor(seconds / 60);
    if (minutes < 1) return t("runtimes.age.lessThanMinute");
    if (minutes < 60) return t("runtimes.age.minutes", { count: minutes });
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return t("runtimes.age.hours", { count: hours });
    return t("runtimes.age.days", { count: Math.floor(hours / 24) });
  };
}

/** One line under the page header that says where the version list came from and why it may be old. */
export function RegistryStatusLine({ status }: { status: RegistryStatus }) {
  const { t } = useTranslation();
  const age = useAge();

  if (status.refreshing) {
    return (
      <p role="status" className="mb-6 flex items-center gap-2 text-[13px] text-muted-foreground">
        <LoaderCircle className="size-4 animate-spin" aria-hidden />
        {t("runtimes.registry.refreshing")}
      </p>
    );
  }

  const problem = status.problem;
  let where: string;
  if (status.source === "fresh") {
    where = t("runtimes.registry.fresh");
  } else if (status.source === "cache") {
    where =
      status.ageSeconds === null
        ? t("runtimes.registry.cacheUnknownAge")
        : t("runtimes.registry.cache", { age: age(status.ageSeconds) });
  } else {
    where = t("runtimes.registry.bundled");
  }

  let why: string | null = null;
  if (problem) {
    if (problem.code === "NETWORK") why = t("runtimes.registry.offline");
    else if (problem.code === "REGISTRY_UNSUPPORTED_SCHEMA")
      why = t("runtimes.registry.needsUpdate");
    else why = t("runtimes.registry.otherProblem", { message: problem.message });
  }

  // Never colour alone: every state has its own icon next to the sentence.
  let Icon = status.source === "fresh" ? CircleCheck : Info;
  if (problem) Icon = problem.code === "NETWORK" ? CloudOff : TriangleAlert;

  return (
    <p role="status" className="mb-6 flex items-start gap-2 text-[13px] text-muted-foreground">
      <Icon
        className={problem ? "mt-0.5 size-4 shrink-0 text-warning" : "mt-0.5 size-4 shrink-0"}
        aria-hidden
      />
      <span>{why ? `${where} ${why}` : where}</span>
    </p>
  );
}
