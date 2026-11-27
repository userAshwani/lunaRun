import {
  CircleAlert,
  CircleCheck,
  FolderOpen,
  HardDrive,
  LoaderCircle,
  Package,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import appIcon from "@/assets/app-icon.png";
import type { HomeChoice } from "@/bindings/HomeChoice";
import type { LunaError } from "@/bindings/LunaError";
import type { OnboardingView } from "@/bindings/OnboardingView";
import type { PackItem } from "@/bindings/PackItem";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { useJobProgressText } from "@/features/jobs/job-progress";
import { useJobs } from "@/features/jobs/use-jobs";
import { runtimeName } from "@/features/projects/project-format";
import { jobKey } from "@/features/runtimes/runtime-jobs";
import { useInstallRuntime } from "@/features/runtimes/use-runtimes";
import { useErrorCategory } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { IpcError, toLunaError } from "@/lib/ipc";
import { pickFolder } from "@/lib/native";
import { cn } from "@/lib/utils";
import {
  useCheckHome,
  useChooseHomeAndRestart,
  useCompleteOnboarding,
  useOnboarding,
} from "./use-onboarding";

const VC_REDIST_URL = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

type Step = "home" | "checks" | "pack";
const STEPS: readonly Step[] = ["home", "checks", "pack"];

function errorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

function reasonOf(error: LunaError): string | null {
  const reason = error.details?.reason;
  return typeof reason === "string" ? reason : null;
}

/** A line with a status icon (and a word for screen readers), what it is, and what to know. */
function CheckLine({
  tone,
  title,
  children,
}: {
  tone: "ok" | "warn" | "problem";
  title: string;
  children?: ReactNode;
}) {
  const { t } = useTranslation();
  const icons: Record<typeof tone, { icon: LucideIcon; className: string; label: string }> = {
    ok: { icon: CircleCheck, className: "text-success", label: t("projects.tone.ok") },
    warn: { icon: TriangleAlert, className: "text-warning", label: t("projects.tone.action") },
    problem: {
      icon: CircleAlert,
      className: "text-destructive",
      label: t("projects.tone.problem"),
    },
  };
  const { icon: Icon, className, label } = icons[tone];
  return (
    <li className="flex items-start gap-3 py-3 first:pt-0 last:pb-0">
      <Icon className={cn("mt-0.5 size-5 shrink-0", className)} aria-hidden />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="font-medium">
          <span className="sr-only">{`${label}: `}</span>
          {title}
        </p>
        {children}
      </div>
    </li>
  );
}

function Problem({ title, text, error }: { title: string; text: string; error: LunaError }) {
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0">
        <p className="text-sm font-medium">{title}</p>
        <p className="text-[13px] text-muted-foreground">{text}</p>
        <p className="mt-1 font-mono text-xs break-words text-muted-foreground">{error.message}</p>
      </div>
    </div>
  );
}

function HomeStep({ view }: { view: OnboardingView }) {
  const { t } = useTranslation();
  const categoryOf = useErrorCategory();
  const check = useCheckHome();
  const choose = useChooseHomeAndRestart();
  const [candidate, setCandidate] = useState<HomeChoice | null>(null);

  const pick = async () => {
    setCandidate(null);
    choose.reset();
    const path = await pickFolder(t("welcome.home.pickTitle"));
    if (!path) return;
    check.mutate(path, {
      onSuccess: (choice) => {
        setCandidate(choice);
      },
    });
  };
  const checkError = check.isError ? errorOf(check.error) : null;
  const chooseError = choose.isError ? errorOf(choose.error) : null;
  const homeAdvice = (error: LunaError) => {
    if (error.code === "BUSY") return t("welcome.home.restartBusy");
    if (reasonOf(error) === "notEmpty") return t("welcome.home.notEmpty");
    if (reasonOf(error) === "notFolder") return t("welcome.home.notFolder");
    return categoryOf(error.code);
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("welcome.home.text")}</p>
      <div className="flex flex-wrap items-center gap-3 rounded-lg border bg-muted/40 px-4 py-3">
        <FolderOpen className="size-5 shrink-0 text-muted-foreground" aria-hidden />
        <code className="min-w-0 flex-1 text-sm break-all">{view.home}</code>
        <Badge variant={view.homeSource === "chosen" ? "info" : "neutral"}>
          {t(
            view.homeSource === "env"
              ? "welcome.home.source.env"
              : view.homeSource === "chosen"
                ? "welcome.home.source.chosen"
                : "welcome.home.source.default",
          )}
        </Badge>
      </div>
      {view.homeSource === "env" && (
        <p className="text-[13px] text-muted-foreground">{t("welcome.home.envNote")}</p>
      )}
      {view.canChooseHome && !candidate && (
        <Button
          variant="outline"
          disabled={check.isPending}
          onClick={() => {
            void pick();
          }}
        >
          {check.isPending ? (
            <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
          ) : (
            <FolderOpen aria-hidden />
          )}
          {t("welcome.home.choose")}
        </Button>
      )}
      {checkError && (
        <Problem
          title={t("welcome.home.problem")}
          text={homeAdvice(checkError)}
          error={checkError}
        />
      )}
      {candidate && (
        <div
          role="group"
          aria-label={t("welcome.home.confirmTitle")}
          className="space-y-3 rounded-lg border p-4"
        >
          <p className="font-medium">{t("welcome.home.confirmTitle")}</p>
          <p className="text-sm break-all text-muted-foreground">
            {t("welcome.home.confirmText", { path: candidate.path })}
          </p>
          {candidate.warnings.map((warning) => (
            <p
              key={warning}
              className="flex items-start gap-2 text-[13px] text-warning-soft-foreground"
            >
              <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
              {t(warning === "synced" ? "welcome.home.warnSynced" : "welcome.home.warnExisting")}
            </p>
          ))}
          {chooseError && (
            <Problem
              title={t("welcome.home.problem")}
              text={homeAdvice(chooseError)}
              error={chooseError}
            />
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={choose.isPending}
              onClick={() => {
                choose.mutate(candidate.path);
              }}
            >
              {choose.isPending && (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
              )}
              {t("welcome.home.useAndRestart")}
            </Button>
            <Button
              variant="outline"
              disabled={choose.isPending}
              onClick={() => {
                setCandidate(null);
                choose.reset();
              }}
            >
              {t("welcome.home.cancel")}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

function ChecksStep({ view }: { view: OnboardingView }) {
  const { t } = useTranslation();
  const { checks, pack } = view;
  const need = pack.items
    .filter((item) => item.installedVersion === null)
    .reduce((sum, item) => sum + (item.size ?? 0), 0);
  const vc = checks.vcRedist;
  return (
    <ul className="divide-y">
      {vc !== "notNeeded" && (
        <CheckLine
          tone={vc === "ok" ? "ok" : vc === "unknown" ? "warn" : "problem"}
          title={t(
            vc === "ok"
              ? "welcome.checks.vcOk"
              : vc === "missing"
                ? "welcome.checks.vcMissing"
                : vc === "outdated"
                  ? "welcome.checks.vcOutdated"
                  : "welcome.checks.vcUnknown",
          )}
        >
          {vc !== "ok" && (
            <>
              <p className="text-[13px] text-muted-foreground">
                {t(vc === "unknown" ? "welcome.checks.vcUnknownWhy" : "welcome.checks.vcWhy")}
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <code className="text-xs break-all text-muted-foreground">{VC_REDIST_URL}</code>
                <CopyButton
                  text={VC_REDIST_URL}
                  label={t("welcome.checks.vcLink")}
                  doneLabel={t("welcome.checks.copied")}
                  failedLabel={t("welcome.checks.copyFailed")}
                  size="sm"
                />
              </div>
            </>
          )}
        </CheckLine>
      )}
      {checks.freeBytes === null ? (
        <CheckLine tone="warn" title={t("welcome.checks.spaceUnknown")} />
      ) : checks.enoughSpace ? (
        <CheckLine
          tone="ok"
          title={t("welcome.checks.spaceOk", { free: formatBytes(checks.freeBytes) })}
        />
      ) : (
        <CheckLine
          tone="problem"
          title={t("welcome.checks.spaceLow", { free: formatBytes(checks.freeBytes) })}
        >
          <p className="text-[13px] text-muted-foreground">
            {t("welcome.checks.spaceLowWhy", { need: formatBytes(need) })}
          </p>
        </CheckLine>
      )}
    </ul>
  );
}

/** One runtime of the pack: chosen or not, and how its install goes once started. */
function PackRow({
  item,
  selected,
  onToggle,
  started,
}: {
  item: PackItem;
  selected: boolean;
  onToggle: (value: boolean) => void;
  started: boolean;
}) {
  const { t } = useTranslation();
  const jobs = useJobs();
  const progress = useJobProgressText();
  const install = useInstallRuntime();
  const job = jobs.data?.find((candidate) => candidate.key === jobKey(item.runtime, item.version));
  const name = t("welcome.pack.itemLabel", {
    name: runtimeName(item.runtime),
    version: item.version,
  });
  const id = `pack-${item.runtime}`;
  const running = job?.state === "running" ? progress(job) : null;

  let status: ReactNode = null;
  if (item.installedVersion !== null) {
    status = (
      <Badge variant="success">
        {t("welcome.pack.installed", { version: item.installedVersion })}
      </Badge>
    );
  } else if (job?.state === "succeeded") {
    status = <Badge variant="success">{t("welcome.pack.done")}</Badge>;
  } else if (job?.state === "failed" || job?.state === "cancelled") {
    status = (
      <div className="flex items-center gap-2">
        <Badge variant="destructive">{t("welcome.pack.failed")}</Badge>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            install.mutate({ runtime: item.runtime, version: item.version });
          }}
        >
          {t("welcome.pack.retry")}
        </Button>
      </div>
    );
  } else if (started && selected && !job) {
    status = <Badge variant="neutral">{t("welcome.pack.queued")}</Badge>;
  }

  return (
    <li className="space-y-2 py-3 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center gap-3">
        <input
          id={id}
          type="checkbox"
          className="size-4 accent-[var(--primary)]"
          checked={item.installedVersion !== null || selected}
          disabled={item.installedVersion !== null || started}
          onChange={(event) => {
            onToggle(event.target.checked);
          }}
        />
        <label htmlFor={id} className="min-w-0 flex-1 font-medium">
          {name}
        </label>
        {item.size !== null && item.installedVersion === null && (
          <span className="text-[13px] text-muted-foreground tabular-nums">
            {formatBytes(item.size)}
          </span>
        )}
        {status}
      </div>
      {running && (
        <div className="space-y-1 pl-7">
          <p className="text-[13px] text-muted-foreground">
            {running.phase}
            {running.bytes ? ` · ${running.bytes}` : ""}
          </p>
          <Progress className="h-1.5" value={running.value} label={name} />
        </div>
      )}
      {job?.state === "failed" && job.error && (
        <p className="pl-7 font-mono text-xs break-words text-muted-foreground">
          {job.error.message}
        </p>
      )}
    </li>
  );
}

function PackStep({
  view,
  selected,
  setSelected,
  started,
}: {
  view: OnboardingView;
  selected: Set<string>;
  setSelected: (next: Set<string>) => void;
  started: boolean;
}) {
  const { t } = useTranslation();
  const { pack } = view;
  const toInstall = pack.items.filter(
    (item) => item.installedVersion === null && selected.has(item.runtime),
  );
  const size = toInstall.reduce((sum, item) => sum + (item.size ?? 0), 0);
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("welcome.pack.text")}</p>
      <ul className="divide-y">
        {pack.items.map((item) => (
          <PackRow
            key={item.runtime}
            item={item}
            started={started}
            selected={selected.has(item.runtime)}
            onToggle={(value) => {
              const next = new Set(selected);
              if (value) next.add(item.runtime);
              else next.delete(item.runtime);
              setSelected(next);
            }}
          />
        ))}
      </ul>
      {pack.unavailable.length > 0 && (
        <p className="text-[13px] text-muted-foreground">
          {t("welcome.pack.unavailable", { names: pack.unavailable.map(runtimeName).join(", ") })}
        </p>
      )}
      {!started && (
        <p className="text-[13px] text-muted-foreground tabular-nums">
          {toInstall.length > 0
            ? t("welcome.pack.total", { size: formatBytes(size) })
            : t("welcome.pack.nothingToDownload")}
        </p>
      )}
    </div>
  );
}

/**
 * The first-run wizard (FR-AP-06, D-044): the data folder (which can be changed only now), a look at this computer, and
 * the Laravel starter pack, installed as ordinary install jobs. It can be skipped at any step; finishing or skipping
 * means it is not shown again.
 */
export function WelcomePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const onboarding = useOnboarding();
  const complete = useCompleteOnboarding();
  const install = useInstallRuntime();
  const [step, setStep] = useState<Step>("home");
  const [selected, setSelected] = useState<Set<string> | null>(null);
  const [started, setStarted] = useState(false);

  const leave = () => {
    complete.mutate(undefined, {
      onSettled: () => {
        void navigate("/", { replace: true });
      },
    });
  };

  const view = onboarding.data;
  const chosen =
    selected ??
    new Set(
      (view?.pack.items ?? [])
        .filter((item) => item.installedVersion === null)
        .map((item) => item.runtime),
    );
  const toInstall = (view?.pack.items ?? []).filter(
    (item) => item.installedVersion === null && chosen.has(item.runtime),
  );
  const index = STEPS.indexOf(step);

  const startInstalls = () => {
    setStarted(true);
    for (const item of toInstall) {
      install.mutate({ runtime: item.runtime, version: item.version });
    }
  };

  let body: ReactNode;
  if (onboarding.isPending) {
    body = (
      <div className="space-y-3" role="status" aria-label={t("welcome.loading")}>
        <Skeleton className="h-5 w-2/3" />
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-9 w-48" />
      </div>
    );
  } else if (onboarding.isError || !view) {
    body = (
      <ErrorState
        error={errorOf(onboarding.error)}
        title={t("welcome.loadError")}
        onRetry={() => {
          void onboarding.refetch();
        }}
      />
    );
  } else if (step === "home") {
    body = <HomeStep view={view} />;
  } else if (step === "checks") {
    body = <ChecksStep view={view} />;
  } else {
    body = (
      <PackStep
        view={view}
        selected={chosen}
        setSelected={(next) => {
          setSelected(next);
        }}
        started={started}
      />
    );
  }

  const titles: Record<Step, string> = {
    home: t("welcome.home.title"),
    checks: t("welcome.checks.title"),
    pack: t("welcome.pack.title"),
  };
  const stepIcons: Record<Step, LucideIcon> = {
    home: FolderOpen,
    checks: HardDrive,
    pack: Package,
  };
  const StepIcon = stepIcons[step];

  return (
    <main className="flex min-h-screen justify-center overflow-y-auto bg-background px-4 py-10">
      <div className="w-full max-w-2xl space-y-6">
        <header className="flex items-start gap-4">
          <img src={appIcon} alt="" width={48} height={48} className="size-12 shrink-0" />
          <div className="min-w-0 flex-1">
            <h1 className="text-2xl font-semibold">{t("welcome.title")}</h1>
            <p className="mt-1 text-sm text-muted-foreground">{t("welcome.description")}</p>
          </div>
          <Button variant="ghost" disabled={complete.isPending} onClick={leave}>
            {t("welcome.skip")}
          </Button>
        </header>

        <ol aria-label={t("welcome.steps.label")} className="flex flex-wrap gap-2">
          {STEPS.map((candidate, position) => (
            <li
              key={candidate}
              aria-current={candidate === step ? "step" : undefined}
              className={cn(
                "flex items-center gap-2 rounded-full border px-3 py-1 text-[13px]",
                candidate === step
                  ? "border-primary/40 bg-primary/10 font-medium text-foreground"
                  : "text-muted-foreground",
              )}
            >
              <span className="tabular-nums">{position + 1}</span>
              {t(`welcome.steps.${candidate}`)}
            </li>
          ))}
        </ol>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <StepIcon className="size-5 text-muted-foreground" aria-hidden />
              {titles[step]}
            </CardTitle>
          </CardHeader>
          <CardContent>{body}</CardContent>
        </Card>

        {view && (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <Button
              variant="outline"
              disabled={index === 0 || started}
              onClick={() => {
                setStep(STEPS[index - 1] ?? "home");
              }}
            >
              {t("welcome.back")}
            </Button>
            {step !== "pack" ? (
              <Button
                onClick={() => {
                  setStep(STEPS[index + 1] ?? "pack");
                }}
              >
                {t("welcome.next")}
              </Button>
            ) : !started && toInstall.length > 0 ? (
              <Button onClick={startInstalls}>
                {toInstall.length === 1
                  ? t("welcome.pack.installOne")
                  : t("welcome.pack.install", { count: toInstall.length })}
              </Button>
            ) : (
              <div className="flex flex-col items-end gap-1">
                <Button disabled={complete.isPending} onClick={leave}>
                  {t("welcome.pack.finish")}
                </Button>
                {started && (
                  <p className="text-[13px] text-muted-foreground">
                    {t("welcome.pack.finishLater")}
                  </p>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </main>
  );
}
