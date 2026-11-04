import { CircleCheck, Info, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ToastContext, type ToastInput } from "@/components/toast-context";
import { Button } from "@/components/ui/button";

interface Toast extends ToastInput {
  id: number;
}

/** How long a confirmation stays. Long enough to read a sentence, short enough not to pile up. */
const LIFETIME_MS = 6000;

/**
 * Confirmations for things that finished ("PHP 8.3.27 installed"). Problems that need an action stay inline
 * where the user can act on them, never in a toast that disappears.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(0);

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const show = useCallback((toast: ToastInput) => {
    nextId.current += 1;
    const entry = { ...toast, id: nextId.current };
    setToasts((current) => [...current.slice(-3), entry]);
  }, []);

  const api = useMemo(() => ({ show }), [show]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div
        role="region"
        aria-label={t("toast.region")}
        className="pointer-events-none fixed right-4 bottom-4 z-50 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2"
      >
        {toasts.map((toast) => (
          <ToastItem key={toast.id} toast={toast} onDismiss={dismiss} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

function ToastItem({ toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  const { t } = useTranslation();
  const Icon = toast.tone === "info" ? Info : CircleCheck;

  useEffect(() => {
    const timer = window.setTimeout(() => {
      onDismiss(toast.id);
    }, LIFETIME_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [toast.id, onDismiss]);

  return (
    <div
      role="status"
      className="pointer-events-auto flex items-start gap-3 rounded-lg border bg-card p-3 text-sm shadow-lg"
    >
      <Icon
        className={
          toast.tone === "info"
            ? "mt-0.5 size-4 shrink-0 text-info"
            : "mt-0.5 size-4 shrink-0 text-success"
        }
        aria-hidden
      />
      <p className="min-w-0 flex-1">{toast.title}</p>
      <Button
        variant="ghost"
        size="icon"
        className="-m-1 size-6"
        aria-label={t("toast.dismiss")}
        onClick={() => {
          onDismiss(toast.id);
        }}
      >
        <X aria-hidden />
      </Button>
    </div>
  );
}
