import { Check, ClipboardCopy, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, type buttonVariants } from "@/components/ui/button";
import type { VariantProps } from "class-variance-authority";

type CopyState = "idle" | "copied" | "failed";

interface CopyButtonProps {
  /** What goes on the clipboard. */
  text: string;
  label: string;
  doneLabel: string;
  failedLabel: string;
  variant?: VariantProps<typeof buttonVariants>["variant"];
  size?: VariantProps<typeof buttonVariants>["size"];
}

/** Copies `text` and says whether it worked, then returns to its normal label. */
export function CopyButton({
  text,
  label,
  doneLabel,
  failedLabel,
  variant = "outline",
  size,
}: CopyButtonProps) {
  const [state, setState] = useState<CopyState>("idle");

  useEffect(() => {
    if (state === "idle") return;
    const timer = window.setTimeout(() => {
      setState("idle");
    }, 2000);
    return () => {
      window.clearTimeout(timer);
    };
  }, [state]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("failed");
    }
  };

  const Icon = state === "copied" ? Check : state === "failed" ? TriangleAlert : ClipboardCopy;
  return (
    <Button
      variant={variant}
      size={size}
      onClick={() => {
        void copy();
      }}
    >
      <Icon aria-hidden />
      <span aria-live="polite">
        {state === "copied" ? doneLabel : state === "failed" ? failedLabel : label}
      </span>
    </Button>
  );
}
