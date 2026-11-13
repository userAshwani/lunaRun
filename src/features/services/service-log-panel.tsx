import { ArrowDownToLine, X } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
import { Tabs } from "@/components/tabs";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { formatLogTime, workerNumber } from "./service-format";
import { useServiceLogs, type LogTab } from "./use-service-logs";

/** How close to the end counts as "following": the last line is in view, give or take a line. */
const FOLLOW_SLACK_PX = 24;

interface ServiceLogPanelProps {
  /** The row whose log is shown. */
  rowId: string;
  /** `MySQL 8.4`. */
  name: string;
  onClose: () => void;
}

/** One log line: its time, and its text with the line breaks it came with. */
function Line({ ts, text }: { ts: number; text: string }) {
  // lunaRun's own notes ("[lunaRun] starting") are muted: the service's words are the ones to read.
  const own = text.startsWith("[lunaRun]");
  return (
    <div className="flex gap-3">
      <span className="shrink-0 text-muted-foreground tabular-nums select-none">
        {formatLogTime(ts)}
      </span>
      <span
        className={own ? "min-w-0 break-words text-muted-foreground" : "min-w-0 break-words"}
        style={{ whiteSpace: "pre-wrap" }}
      >
        {text}
      </span>
    </div>
  );
}

/** The lines of one tab in a scroll box that follows the end until the user scrolls up. */
function LogLines({ tab }: { tab: LogTab }) {
  const { t } = useTranslation();
  const box = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  const count = tab.lines.length;

  // After every change of the lines, stay at the end, unless the user has scrolled up to read.
  useLayoutEffect(() => {
    const element = box.current;
    if (following && element) element.scrollTop = element.scrollHeight;
  }, [count, following]);

  const copy = tab.lines.map((line) => `${formatLogTime(line.ts)} ${line.line}`).join("\n");

  return (
    <div>
      <div className="relative">
        <div
          ref={box}
          role="log"
          tabIndex={0}
          aria-label={t("services.logs.logLabel")}
          onScroll={(event) => {
            const element = event.currentTarget;
            setFollowing(
              element.scrollHeight - element.scrollTop - element.clientHeight <= FOLLOW_SLACK_PX,
            );
          }}
          className="h-72 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs leading-5 focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
        >
          {count === 0 ? (
            <p className="font-sans text-[13px] text-muted-foreground">
              {t("services.logs.empty")}
            </p>
          ) : (
            tab.lines.map((line, index) => (
              // Lines have no id of their own; the list only grows at the end or is replaced as a whole.
              <Line key={index} ts={line.ts} text={line.line} />
            ))
          )}
        </div>
        {!following && (
          <Button
            variant="outline"
            size="sm"
            className="absolute right-4 bottom-3 shadow-sm"
            onClick={() => {
              setFollowing(true);
            }}
          >
            <ArrowDownToLine aria-hidden />
            {t("services.logs.jump")}
          </Button>
        )}
      </div>
      <div className="mt-3 flex items-center justify-between gap-3">
        <p className="text-[13px] text-muted-foreground tabular-nums">
          {t("services.logs.count", { count })}
        </p>
        <CopyButton
          text={copy}
          size="sm"
          label={t("services.logs.copy")}
          doneLabel={t("services.logs.copied")}
          failedLabel={t("services.logs.copyFailed")}
        />
      </div>
    </div>
  );
}

/**
 * The live log of one service row, a tab per source: what the service prints, then its own log files (the MySQL error
 * log, PHP-FPM's log). A Windows pool has the output of each of its workers.
 */
export function ServiceLogPanel({ rowId, name, onClose }: ServiceLogPanelProps) {
  const { t } = useTranslation();
  const logs = useServiceLogs(rowId);
  const [selected, setSelected] = useState<string | null>(null);
  const many = new Set(logs.tabs.map((tab) => tab.service)).size > 1;
  const current = logs.tabs.find((tab) => tab.id === selected) ?? logs.tabs[0];

  const labelOf = (tab: LogTab) => {
    const source = tab.source === "output" ? t("services.logs.output") : tab.source;
    const worker = workerNumber(tab.service);
    return many && worker !== null ? t("services.logs.worker", { number: worker, source }) : source;
  };

  return (
    <Card>
      <CardContent>
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-base font-semibold">{t("services.logs.title", { name })}</h2>
            <p className="text-[13px] text-muted-foreground">{t("services.logs.description")}</p>
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="-m-1 size-8 shrink-0"
            aria-label={t("services.logs.closeLabel", { name })}
            onClick={onClose}
          >
            <X aria-hidden />
          </Button>
        </div>
        {logs.isError && logs.error ? (
          <ErrorState
            error={logs.error}
            title={t("services.logs.loadError")}
            onRetry={logs.retry}
          />
        ) : logs.isLoading ? (
          <div role="status" aria-label={t("services.logs.loading")} className="space-y-2">
            {[0, 1, 2, 3].map((row) => (
              <Skeleton key={row} className="h-4 w-full" />
            ))}
          </div>
        ) : !current ? (
          <p className="text-sm text-muted-foreground">{t("services.logs.neverStarted")}</p>
        ) : (
          <Tabs
            label={t("services.logs.tabsLabel", { name })}
            value={current.id}
            onChange={setSelected}
            items={logs.tabs.map((tab) => ({ id: tab.id, label: labelOf(tab) }))}
          >
            <LogLines key={current.id} tab={current} />
          </Tabs>
        )}
      </CardContent>
    </Card>
  );
}
