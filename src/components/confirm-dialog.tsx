import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";

interface ConfirmDialogProps {
  /** Names what is about to happen and to what ("Uninstall PHP 8.3.27?"). */
  title: string;
  /** What the choice means, in one or two sentences. */
  description?: ReactNode;
  /** Extra content between the description and the buttons: lists, options, warnings. */
  children?: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  /** The action cannot be undone: the confirm button turns red. */
  destructive?: boolean;
  /** Keeps the confirm button off, for example while what the dialog needs to say is still loading. */
  confirmDisabled?: boolean;
  /** The action is under way: both buttons stay off so it cannot be started twice. */
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

const FOCUSABLE =
  'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';

/**
 * A modal confirmation. Focus starts on Cancel (the safe choice for something destructive), Tab stays inside,
 * Escape and a click outside cancel, and focus returns to the control that opened it.
 */
export function ConfirmDialog({
  title,
  description,
  children,
  confirmLabel,
  cancelLabel,
  destructive = false,
  confirmDisabled = false,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const titleId = useId();
  const descriptionId = useId();
  const panel = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    cancel.current?.focus();
    return () => {
      opener?.focus();
    };
  }, []);

  // On the document, not the panel: after a click on plain text inside the dialog focus sits on the page
  // behind it, and Escape must still close the dialog.
  useEffect(() => {
    const onEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onCancel();
    };
    document.addEventListener("keydown", onEscape);
    return () => {
      document.removeEventListener("keydown", onEscape);
    };
  }, [busy, onCancel]);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Tab" || !panel.current) return;
    const items = [...panel.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onMouseDown={(event) => {
        // Only a press on the backdrop itself, not one that started inside the panel and ended outside.
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        onKeyDown={onKeyDown}
        className="flex max-h-full w-full max-w-md flex-col gap-4 overflow-y-auto rounded-xl border bg-card p-6 text-card-foreground shadow-xl"
      >
        <div>
          <h2 id={titleId} className="text-base leading-tight font-semibold">
            {title}
          </h2>
          {description && (
            <div id={descriptionId} className="mt-2 text-sm text-muted-foreground">
              {description}
            </div>
          )}
        </div>
        {children}
        <div className="flex justify-end gap-2 pt-2">
          <Button ref={cancel} variant="outline" disabled={busy} onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button
            variant={destructive ? "destructive" : "default"}
            disabled={confirmDisabled || busy}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
