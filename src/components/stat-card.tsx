import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

interface StatCardProps {
  icon: LucideIcon;
  label: string;
  value: number | string;
  hint: string;
  /** The value is still being read: a skeleton instead of a number that is not true yet. */
  loading?: boolean;
}

export function StatCard({ icon: Icon, label, value, hint, loading }: StatCardProps) {
  return (
    <Card className="flex items-start justify-between gap-4 p-5">
      <div className="min-w-0">
        <p className="text-sm font-medium text-muted-foreground">{label}</p>
        {loading ? (
          <>
            <Skeleton className="mt-2 h-7 w-10" />
            <Skeleton className="mt-2 h-4 w-28" />
          </>
        ) : (
          <>
            <p className="mt-2 text-3xl leading-none font-semibold tracking-tight tabular-nums">
              {value}
            </p>
            <p className="mt-2 text-sm text-muted-foreground">{hint}</p>
          </>
        )}
      </div>
      <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-primary-soft-foreground">
        <Icon className="size-5" aria-hidden />
      </div>
    </Card>
  );
}
