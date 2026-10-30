import * as React from "react";

import { cn } from "@/lib/utils";

/** Placeholder block shown while data loads. Size it with className. */
function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div aria-hidden className={cn("animate-pulse rounded-md bg-muted", className)} {...props} />
  );
}

export { Skeleton };
