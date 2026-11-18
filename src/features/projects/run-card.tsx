import {
  CircleAlert,
  CircleCheck,
  CircleStop,
  Code,
  ExternalLink,
  FolderOpen,
  LoaderCircle,
  Play,
  Square,
  SquareTerminal,
  TriangleAlert,
  X,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { DatabaseSettings } from "@/bindings/DatabaseSettings";
import type { LunaError } from "@/bindings/LunaError";
import type { OpenTarget } from "@/bindings/OpenTarget";
import type { Project } from "@/bindings/Project";
import type { SiteEntry } from "@/bindings/SiteEntry";
import { CopyButton } from "@/components/copy-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { useJobProgressText } from "@/features/jobs/job-progress";
import { useErrorCategory } from "@/lib/errors";
import { IpcError, toLunaError } from "@/lib/ipc";
import {
  runActionOf,
  useOpenProject,
  useProjectDatabase,
  useProjectRun,
  type ProjectRun,
} from "./use-project-run";

type Shown = "running" | "stopped" | "starting" | "stopping" | "notAnswering";

/** What the badge says: the services decide, and a job under way says what it is doing. */
function shownState(entry: SiteEntry | undefined, run: ProjectRun): Shown {
  if (run.job?.state === "running")
    return runActionOf(run.job) === "stop" ? "stopping" : "starting";
  if (entry?.running) return "running";
  if (entry?.served) return "notAnswering";
  return "stopped";
}

const LOOK = {
  running: { icon: CircleCheck, variant: "success" },
  stopped: { icon: CircleStop, variant: "neutral" },
  starting: { icon: LoaderCircle, variant: "info", spins: true },
  stopping: { icon: LoaderCircle, variant: "warning", spins: true },
  notAnswering: { icon: TriangleAlert, variant: "warning" },
} as const;

/** The state of a project as a badge: an icon and a word, never colour alone. */
export function RunStateBadge({ state }: { state: Shown }) {
  const { t } = useTranslation();
  const look = LOOK[state];
  const Icon = look.icon;
  const spins = "spins" in look && look.spins;
  return (
    <Badge variant={look.variant}>
      <Icon
        className={spins ? "size-3 animate-spin motion-reduce:animate-none" : "size-3"}
        aria-hidden
      />
      {t(`projects.run.status.${state}`)}
    </Badge>
  );
}

function details(error: LunaError, key: string): string | null {
  const value = error.details?.[key];
  return typeof value === "string" || typeof value === "number" ? String(value) : null;
}

/** What went wrong with a start or a stop, in plain words, with what to do; the technical message underneath. */
function RunProblem({
  problem,
  name,
  onDismiss,
}: {
  problem: { error: LunaError; action: "start" | "stop" };
  name: string;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const { error, action } = problem;
  let advice: string | null = null;
  if (error.code === "PORT_IN_USE") {
    const port = details(error, "port") ?? "";
    const process = details(error, "processName");
    advice = process
      ? t("projects.run.problem.portInUse", { port, process })
      : t("projects.run.problem.portInUseUnknown", { port });
  } else if (error.code === "NO_SITE") {
    advice =
      details(error, "reason") === "folder"
        ? t("projects.run.problem.noSiteFolder")
        : t("projects.run.problem.noSitePhp");
  } else if (error.code === "NOT_FOUND") {
    advice = t("projects.run.problem.notInstalled");
  }
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-xl border border-destructive/30 bg-destructive/5 px-4 py-3"
    >
      <CircleAlert className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium">
          {t(
            action === "start"
              ? "projects.run.problem.startTitle"
              : "projects.run.problem.stopTitle",
            {
              name,
            },
          )}
        </p>
        <p className="mt-1 text-[13px] text-muted-foreground">{advice ?? categoryOf(error.code)}</p>
        <p className="mt-2 rounded-md bg-muted px-3 py-2 font-mono text-xs break-words text-muted-foreground">
          {error.message}
        </p>
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={onDismiss}
        aria-label={t("projects.run.problem.dismiss")}
      >
        <X aria-hidden />
      </Button>
    </div>
  );
}

/** What `.env` would hold to connect to the database lunaRun runs: no password, because there is none. */
function databaseEnv(settings: DatabaseSettings): string {
  const lines = [
    `DB_CONNECTION=${settings.runtime}`,
    `DB_HOST=${settings.host}`,
    `DB_PORT=${String(settings.port)}`,
  ];
  if (settings.database) lines.push(`DB_DATABASE=${settings.database}`);
  lines.push(`DB_USERNAME=${settings.username}`, "DB_PASSWORD=");
  return `${lines.join("\n")}\n`;
}

interface RunCardProps {
  project: Project;
  /** This project's entry of `sites_list`, while it is there. */
  entry: SiteEntry | undefined;
  /** The sites are still being read. */
  loading: boolean;
}

/**
 * Starting and stopping a project, its state, and the quick actions: open the site, the folder, the editor, and copy
 * the settings of the database. The state is read from the services; Start says plainly why it cannot work when there
 * is no site.
 */
export function RunCard({ project, entry, loading }: RunCardProps) {
  const { t } = useTranslation();
  const run = useProjectRun(project.id);
  const open = useOpenProject();
  const database = useProjectDatabase(project.id).data;
  const progress = useJobProgressText();
  const [openProblem, setOpenProblem] = useState<LunaError | null>(null);
  const categoryOf = useErrorCategory();
  const state = shownState(entry, run);
  const canStart = entry?.state === "ready";
  const stopping = run.job?.state === "running" && runActionOf(run.job) === "stop";
  const jobProgress = run.job?.state === "running" ? progress(run.job) : null;

  const openIt = (target: OpenTarget) => {
    setOpenProblem(null);
    open.mutate(
      { id: project.id, target },
      {
        onError: (thrown) => {
          setOpenProblem(thrown instanceof IpcError ? thrown.error : toLunaError(thrown));
        },
      },
    );
  };

  /** What to do about a quick action that failed, when lunaRun knows: the rest falls back to the category of the error. */
  const openAdvice = (problem: LunaError): string | null => {
    const target = details(problem, "target");
    if (problem.code === "NOT_FOUND" && target === "editor") {
      return t("projects.run.open.editorMissing");
    }
    if (problem.code === "NOT_FOUND" && target === "terminal") {
      return t("projects.run.open.terminalMissing");
    }
    if (problem.code === "NO_SITE" && details(problem, "reason") === "php") {
      return t("projects.run.open.terminalNoPhp");
    }
    return null;
  };

  // One literal key per state: a key built from a template string cannot be checked by the types of i18next 27, because
  // the text of "running" takes an option the others do not.
  let explain: string;
  if (state === "running") {
    // A running project always has its address (it has a site); without one there is nothing true to say about it.
    explain = entry?.address ? t("projects.run.explain.running", { address: entry.address }) : "";
  } else if (state === "stopped") {
    explain =
      !canStart && entry
        ? t(entry.state === "noPhp" ? "projects.site.noPhp" : "projects.site.folderMissing")
        : t("projects.run.explain.stopped");
  } else if (state === "starting") {
    explain = t("projects.run.explain.starting");
  } else if (state === "stopping") {
    explain = t("projects.run.explain.stopping");
  } else {
    explain = t("projects.run.explain.notAnswering");
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("projects.run.title")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-3">
          {loading && !entry ? (
            <Skeleton className="h-6 w-24" aria-label={t("projects.site.loading")} />
          ) : (
            <RunStateBadge state={state} />
          )}
          <p className="min-w-0 flex-1 text-sm text-muted-foreground">{explain}</p>
          <div className="flex shrink-0 items-center gap-2">
            {run.job?.state === "running" && !stopping ? (
              <Button
                variant="outline"
                aria-label={t("projects.run.cancelLabel", { name: project.name })}
                onClick={run.cancel}
              >
                <X aria-hidden />
                {t("projects.run.cancel")}
              </Button>
            ) : entry?.served || stopping ? (
              <Button
                variant="outline"
                disabled={run.busy}
                aria-label={t("projects.run.stopLabel", { name: project.name })}
                onClick={run.stop}
              >
                <Square aria-hidden />
                {t("projects.run.stop")}
              </Button>
            ) : (
              <Button
                disabled={run.busy || !canStart}
                aria-label={t("projects.run.startLabel", { name: project.name })}
                onClick={run.start}
              >
                {run.busy ? (
                  <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
                ) : (
                  <Play aria-hidden />
                )}
                {t("projects.run.start")}
              </Button>
            )}
          </div>
        </div>

        {jobProgress && (
          <div className="max-w-md space-y-1">
            <p className="text-[13px] text-muted-foreground">{jobProgress.phase}</p>
            <Progress
              className="h-1.5"
              value={jobProgress.value}
              label={t("projects.run.progressLabel", { name: project.name })}
            />
          </div>
        )}

        {run.problem && (
          <RunProblem problem={run.problem} name={project.name} onDismiss={run.dismiss} />
        )}

        <div className="space-y-2 border-t pt-4">
          <div
            role="group"
            aria-label={t("projects.run.actions.label")}
            className="flex flex-wrap items-center gap-2"
          >
            <Button
              variant="outline"
              size="sm"
              disabled={!entry?.address || open.isPending}
              title={entry?.address ? undefined : t("projects.run.actions.browserNeedsSite")}
              onClick={() => {
                openIt("browser");
              }}
            >
              <ExternalLink aria-hidden />
              {t("projects.run.actions.browser")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={open.isPending}
              onClick={() => {
                openIt("folder");
              }}
            >
              <FolderOpen aria-hidden />
              {t("projects.run.actions.folder")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={open.isPending}
              onClick={() => {
                openIt("editor");
              }}
            >
              <Code aria-hidden />
              {t("projects.run.actions.editor")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={open.isPending}
              title={t("projects.run.actions.terminalHint")}
              onClick={() => {
                openIt("terminal");
              }}
            >
              <SquareTerminal aria-hidden />
              {t("projects.run.actions.terminal")}
            </Button>
            {database && (
              <CopyButton
                text={databaseEnv(database)}
                label={t("projects.run.actions.database")}
                doneLabel={t("projects.run.actions.databaseCopied")}
                failedLabel={t("projects.run.actions.databaseFailed")}
                size="sm"
              />
            )}
          </div>
          {database && (
            <p className="text-[13px] text-muted-foreground">
              {t("projects.run.actions.databaseHint", { port: database.port })}
            </p>
          )}
          {openProblem && (
            <div
              role="alert"
              className="flex items-start gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2"
            >
              <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">{t("projects.run.open.title")}</p>
                <p className="text-[13px] text-muted-foreground">
                  {openAdvice(openProblem) ?? categoryOf(openProblem.code)}
                </p>
                <p className="mt-1 font-mono text-xs break-words text-muted-foreground">
                  {openProblem.message}
                </p>
              </div>
              <Button
                variant="ghost"
                size="sm"
                aria-label={t("projects.run.open.dismiss")}
                onClick={() => {
                  setOpenProblem(null);
                }}
              >
                <X aria-hidden />
              </Button>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
