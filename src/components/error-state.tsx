import { TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { LunaError } from "@/bindings/LunaError";
import { Button } from "@/components/ui/button";

interface ErrorStateProps {
  error: LunaError;
  /** What failed, in the user's words ("Could not load system information"). */
  title: string;
  onRetry?: () => void;
}

/** Full error state: what failed, the category, the technical detail, and how to retry. */
export function ErrorState({ error, title, onRetry }: ErrorStateProps) {
  const { t, i18n } = useTranslation();
  // A code this build does not know (newer backend) must still render something sensible.
  const known = i18n.exists(`errors.${error.code}`);
  const category = known ? t(`errors.${error.code}`) : t("errors.INTERNAL");

  return (
    <div
      role="alert"
      className="flex flex-col items-center rounded-xl border bg-card px-6 py-12 text-center shadow-xs"
    >
      <div className="flex size-12 items-center justify-center rounded-full bg-destructive/10 text-destructive">
        <TriangleAlert className="size-6" aria-hidden />
      </div>
      <h2 className="mt-4 text-base font-semibold">{title}</h2>
      <p className="mt-1 text-sm font-medium text-muted-foreground">{category}</p>
      <p className="mt-3 max-w-lg rounded-md bg-muted px-3 py-2 font-mono text-xs break-words text-muted-foreground">
        {error.message}
      </p>
      {onRetry && (
        <Button className="mt-5" variant="outline" onClick={onRetry}>
          {t("common.retry")}
        </Button>
      )}
    </div>
  );
}
