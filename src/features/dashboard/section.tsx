import { TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { LunaError } from "@/bindings/LunaError";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useErrorCategory } from "@/lib/errors";

interface SectionProps {
  title: string;
  description: string;
  /** A link to the page where the rest is, on the right of the title. */
  action?: ReactNode;
  children: ReactNode;
}

/** One card of the Dashboard: a title, one line saying what it is, a link to its page, and the rows. */
export function Section({ title, description, action, children }: SectionProps) {
  return (
    <Card className="min-w-0">
      <CardHeader className="flex-row items-start justify-between gap-4 pb-1">
        <div className="min-w-0">
          <CardTitle>{title}</CardTitle>
          <CardDescription className="mt-1">{description}</CardDescription>
        </div>
        {action && <div className="shrink-0">{action}</div>}
      </CardHeader>
      {children}
    </Card>
  );
}

/** A card whose data could not be read: what failed, why in plain words, the technical detail, and Try again. */
export function SectionError({
  error,
  title,
  onRetry,
}: {
  error: LunaError;
  title: string;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  const category = useErrorCategory()(error.code);
  return (
    <div role="alert" className="flex items-start gap-3 px-6 pt-3 pb-5">
      <TriangleAlert className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{title}</p>
        <p className="mt-0.5 text-[13px] text-muted-foreground">{category}</p>
        <p className="mt-2 rounded-md bg-muted px-3 py-2 font-mono text-xs break-words text-muted-foreground">
          {error.message}
        </p>
        <Button className="mt-3" variant="outline" size="sm" onClick={onRetry}>
          {t("common.retry")}
        </Button>
      </div>
    </div>
  );
}

/** Rows that are still being read: the shape of the list, not a spinner. */
export function SectionSkeleton({ label, rows = 3 }: { label: string; rows?: number }) {
  return (
    <div role="status" aria-label={label} className="divide-y px-6 pb-2">
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="flex items-center gap-3 py-3">
          <div className="min-w-0 flex-1 space-y-2">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-3 w-48" />
          </div>
          <Skeleton className="h-6 w-20" />
        </div>
      ))}
    </div>
  );
}
