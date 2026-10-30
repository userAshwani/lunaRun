import * as React from "react";

import { cn } from "@/lib/utils";

interface ProgressProps extends Omit<React.ComponentProps<"div">, "children"> {
  /** 0 to 100, or `null` when the total is unknown (shows an indeterminate bar). */
  value: number | null;
  /** What is progressing, read by screen readers. */
  label: string;
}

function Progress({ value, label, className, ...props }: ProgressProps) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value ?? undefined}
      className={cn("h-2 w-full overflow-hidden rounded-full bg-muted", className)}
      {...props}
    >
      <div
        className={cn(
          "h-full rounded-full bg-primary transition-[width] duration-200 motion-reduce:transition-none",
          value === null && "w-full animate-pulse opacity-40 motion-reduce:animate-none",
        )}
        style={value === null ? undefined : { width: `${String(value)}%` }}
      />
    </div>
  );
}

export { Progress };
