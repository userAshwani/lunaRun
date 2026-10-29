import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";

interface StatCardProps {
  icon: LucideIcon;
  label: string;
  value: number | string;
  hint: string;
}

export function StatCard({ icon: Icon, label, value, hint }: StatCardProps) {
  return (
    <Card className="flex items-start justify-between gap-4 p-5">
      <div className="min-w-0">
        <p className="text-sm font-medium text-muted-foreground">{label}</p>
        <p className="mt-2 text-3xl leading-none font-semibold tracking-tight tabular-nums">
          {value}
        </p>
        <p className="mt-2 text-sm text-muted-foreground">{hint}</p>
      </div>
      <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-primary-soft-foreground">
        <Icon className="size-5" aria-hidden />
      </div>
    </Card>
  );
}
