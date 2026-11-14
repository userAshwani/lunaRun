import { CircleAlert, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { LunaError } from "@/bindings/LunaError";
import { Button } from "@/components/ui/button";
import { useErrorCategory } from "@/lib/errors";

interface AddProblemProps {
  error: LunaError;
  /** The folder that was being added, as the user gave it. */
  path: string;
  onDismiss: () => void;
}

function textAt(details: LunaError["details"], key: string): string | null {
  const value = details?.[key];
  return typeof value === "string" && value !== "" ? value : null;
}

/**
 * Why a folder was not added, in plain words. A folder that is already a project says so and links to it; anything
 * else names the category and keeps the technical message underneath for a bug report.
 */
export function AddProblem({ error, path, onDismiss }: AddProblemProps) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const existingId = error.code === "PROJECT_EXISTS" ? textAt(error.details, "id") : null;
  const existingName = textAt(error.details, "name") ?? path;

  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-xl border border-destructive/30 bg-destructive/5 px-4 py-3"
    >
      <CircleAlert className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{t("projects.add.failedTitle")}</p>
        {existingId ? (
          <p className="mt-1 text-[13px] text-muted-foreground">
            {t("projects.add.alreadyAdded", { name: existingName })}{" "}
            <Link to={`/projects/${existingId}`} className="font-medium text-primary underline">
              {t("projects.add.openExisting", { name: existingName })}
            </Link>
          </p>
        ) : (
          <>
            <p className="mt-1 text-[13px] text-muted-foreground">{categoryOf(error.code)}</p>
            <p className="mt-2 rounded-md bg-muted px-3 py-2 font-mono text-xs break-words text-muted-foreground">
              {error.message}
            </p>
          </>
        )}
      </div>
      <Button variant="ghost" size="sm" onClick={onDismiss} aria-label={t("projects.add.dismiss")}>
        <X aria-hidden />
      </Button>
    </div>
  );
}
