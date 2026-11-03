import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * A data table. Wrap it in a scroll container (`TableScroll`) when it can get long, so the header stays
 * visible and the region can be scrolled with the keyboard.
 */
function Table({ className, ...props }: React.ComponentProps<"table">) {
  return <table className={cn("w-full caption-bottom text-sm", className)} {...props} />;
}

function TableScroll({
  className,
  label,
  ...props
}: React.ComponentProps<"div"> & { label: string }) {
  return (
    <div
      role="region"
      aria-label={label}
      // Scrollable regions must be reachable by keyboard.
      tabIndex={0}
      className={cn(
        "max-h-[26rem] overflow-auto rounded-md outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
        className,
      )}
      {...props}
    />
  );
}

function TableHeader({ className, ...props }: React.ComponentProps<"thead">) {
  return <thead className={cn("sticky top-0 z-10 bg-card", className)} {...props} />;
}

function TableBody({ className, ...props }: React.ComponentProps<"tbody">) {
  return <tbody className={cn("[&_tr:last-child]:border-0", className)} {...props} />;
}

function TableRow({ className, ...props }: React.ComponentProps<"tr">) {
  return (
    <tr className={cn("border-b transition-colors hover:bg-muted/50", className)} {...props} />
  );
}

function TableHead({ className, ...props }: React.ComponentProps<"th">) {
  return (
    <th
      className={cn(
        "h-10 border-b px-4 text-left align-middle text-xs font-medium whitespace-nowrap text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function TableCell({ className, ...props }: React.ComponentProps<"td">) {
  return <td className={cn("px-4 py-3 align-middle", className)} {...props} />;
}

export { Table, TableScroll, TableHeader, TableBody, TableRow, TableHead, TableCell };
