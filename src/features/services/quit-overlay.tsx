import { CircleCheck, LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { ShutdownGroup } from "@/bindings/ShutdownGroup";
import type { ShutdownProgress } from "@/bindings/ShutdownProgress";
import { Progress } from "@/components/ui/progress";
import { onShutdownProgress } from "@/lib/events";

/** The order the backend stops them in: nothing sends requests to PHP, then nothing sends queries to a database. */
const ORDER: readonly ShutdownGroup[] = ["web", "php", "databases"];

/**
 * Shown over everything while the app stops its services before it quits. The window stays open for that (a stop can take
 * up to a minute per group), and this says why: which group is being stopped, how far it has come, and what it waits for.
 * It appears with the first `shutdown:progress` event, so a quit with nothing running never shows it.
 */
export function QuitOverlay() {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<ReadonlyMap<ShutdownGroup, ShutdownProgress>>(new Map());

  useEffect(() => {
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;
    void onShutdownProgress((progress) => {
      setGroups((current) => new Map(current).set(progress.group, progress));
    }).then((off) => {
      if (lifecycle.disposed) off();
      else unsubscribe = off;
    });
    return () => {
      lifecycle.disposed = true;
      unsubscribe?.();
    };
  }, []);

  if (groups.size === 0) return null;
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-4">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="quit-title"
        aria-describedby="quit-description"
        className="w-full max-w-md rounded-xl border bg-card p-6 shadow-lg"
      >
        <h2 id="quit-title" className="text-base font-semibold">
          {t("quit.title")}
        </h2>
        <p id="quit-description" className="mt-1 text-sm text-muted-foreground">
          {t("quit.description")}
        </p>
        <ul className="mt-5 space-y-4" aria-live="polite">
          {ORDER.filter((group) => groups.has(group)).map((group) => {
            const progress = groups.get(group);
            if (!progress) return null;
            const finished = progress.done >= progress.total;
            return (
              <li key={group}>
                <div className="flex items-center justify-between gap-3 text-sm">
                  <span className="flex items-center gap-2 font-medium">
                    {finished ? (
                      <CircleCheck className="size-4 text-success" aria-hidden />
                    ) : (
                      <LoaderCircle
                        className="size-4 animate-spin text-muted-foreground motion-reduce:animate-none"
                        aria-hidden
                      />
                    )}
                    {t(`quit.group.${group}`)}
                    {finished && <span className="sr-only">{t("quit.done")}</span>}
                  </span>
                  <span className="text-[13px] text-muted-foreground tabular-nums">
                    {t("quit.progress", { done: progress.done, total: progress.total })}
                  </span>
                </div>
                <Progress
                  className="mt-2 h-1.5"
                  value={
                    progress.total === 0 ? 100 : Math.round((progress.done / progress.total) * 100)
                  }
                  label={t("quit.groupLabel", { group: t(`quit.group.${group}`) })}
                />
                {!finished && progress.remaining.length > 0 && (
                  <p className="mt-1.5 text-[13px] text-muted-foreground">
                    {t("quit.waiting", { services: progress.remaining.join(", ") })}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      </div>
    </div>,
    document.body,
  );
}
