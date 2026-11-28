import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description: string;
  /** The one action that fixes the emptiness, if there is one. */
  action?: ReactNode;
  /** Inside a card: less room and no frame of its own. */
  compact?: boolean;
}

export function EmptyState({ icon: Icon, title, description, action, compact }: EmptyStateProps) {
  const Heading = compact ? "h3" : "h2";
  return (
    <div
      className={cn(
        "flex flex-col items-center text-center",
        compact ? "px-4 py-6" : "rounded-xl border border-dashed bg-card px-6 py-16",
      )}
    >
      <div
        className={cn(
          "flex items-center justify-center rounded-full bg-primary-soft text-primary-soft-foreground",
          compact ? "size-10" : "size-12",
        )}
      >
        <Icon className={compact ? "size-5" : "size-6"} aria-hidden />
      </div>
      {/* Inside a card the card's title is the h2. */}
      <Heading className={cn("text-base font-semibold", compact ? "mt-3" : "mt-4")}>
        {title}
      </Heading>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">{description}</p>
      {action && <div className={compact ? "mt-4" : "mt-5"}>{action}</div>}
    </div>
  );
}
