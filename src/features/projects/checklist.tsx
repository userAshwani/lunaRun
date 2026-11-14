import { CircleAlert, CircleCheck, Info, TriangleAlert, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { Availability } from "@/bindings/Availability";
import type { DatabaseSuggestion } from "@/bindings/DatabaseSuggestion";
import type { DbHint } from "@/bindings/DbHint";
import type { ExtensionStatus } from "@/bindings/ExtensionStatus";
import type { Project } from "@/bindings/Project";
import type { Requirement } from "@/bindings/Requirement";
import type { RuntimeResolution } from "@/bindings/RuntimeResolution";
import type { Warning } from "@/bindings/Warning";
import { cn } from "@/lib/utils";
import { InstallRuntime } from "./install-runtime";
import { runtimeName, runtimeState } from "./project-format";

export type Tone = "ok" | "action" | "problem" | "info";

type ToneLabel =
  "projects.tone.ok" | "projects.tone.action" | "projects.tone.problem" | "projects.tone.info";

const TONE: Readonly<Record<Tone, { icon: LucideIcon; className: string; label: ToneLabel }>> = {
  ok: { icon: CircleCheck, className: "text-success", label: "projects.tone.ok" },
  action: { icon: TriangleAlert, className: "text-warning", label: "projects.tone.action" },
  problem: { icon: CircleAlert, className: "text-destructive", label: "projects.tone.problem" },
  info: { icon: Info, className: "text-muted-foreground", label: "projects.tone.info" },
};

interface ChecklistRowProps {
  tone: Tone;
  title: string;
  children?: ReactNode;
  /** The one thing that fixes it, at the right of the row. */
  action?: ReactNode;
}

/** One line of the checklist: a status icon (with a word for screen readers), what it is, and what can be done. */
export function ChecklistRow({ tone, title, children, action }: ChecklistRowProps) {
  const { t } = useTranslation();
  const { icon: Icon, className, label } = TONE[tone];
  return (
    <li className="flex items-start gap-3 py-4 first:pt-0 last:pb-0">
      <Icon className={cn("mt-0.5 size-5 shrink-0", className)} aria-hidden />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="font-medium">
          <span className="sr-only">{`${t(label)}: `}</span>
          {title}
        </p>
        {children}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </li>
  );
}

function Detail({ children, warn = false }: { children: ReactNode; warn?: boolean }) {
  return (
    <p
      className={cn("text-[13px]", warn ? "text-warning-soft-foreground" : "text-muted-foreground")}
    >
      {children}
    </p>
  );
}

interface RuntimeRowProps {
  runtime: "php" | "node";
  requirement: Requirement | null;
  resolution: RuntimeResolution;
  project: Project;
}

/** PHP or Node: what will be used, why, what is wrong with it, and where the requirement came from. */
export function RuntimeRow({ runtime, requirement, resolution, project }: RuntimeRowProps) {
  const { t } = useTranslation();
  const name = runtimeName(runtime);
  const state = runtimeState(resolution);
  const chosen = resolution.recommended;
  const warnings = {
    eol: chosen?.eol === true,
    rosetta: chosen?.rosettaFallback === true,
    conflict: resolution.conflictsWith !== undefined,
  };
  const tone: Tone =
    state.kind === "none"
      ? "problem"
      : state.kind === "install" || warnings.eol || warnings.conflict
        ? "action"
        : "ok";
  const kind = state.kind === "notRequired" ? "none" : state.kind;
  const title = t(`projects.check.${runtime}.${kind}`, {
    version: state.kind === "ready" || state.kind === "install" ? state.version : "",
  });
  const others = resolution.installedMatches.filter((version) => version !== chosen?.version);

  return (
    <ChecklistRow
      tone={tone}
      title={title}
      action={
        state.kind === "install" ? (
          <InstallRuntime runtime={runtime} version={state.version} projectName={project.name} />
        ) : undefined
      }
    >
      <Detail>
        {t(`projects.basis.${resolution.basis}`, { file: requirement?.pin?.file ?? "" })}
        {resolution.constraint &&
          resolution.basis !== "manual" &&
          ` ${t("projects.check.asks", { constraint: resolution.constraint })}`}
      </Detail>
      {warnings.conflict && resolution.conflictsWith !== undefined && (
        <Detail warn>
          {t("projects.check.conflict", { constraint: resolution.conflictsWith })}
        </Detail>
      )}
      {warnings.eol && <Detail warn>{t("projects.check.eol")}</Detail>}
      {warnings.rosetta && <Detail warn>{t("projects.check.rosetta")}</Detail>}
      {resolution.issues.map((issue) => (
        <Detail key={issue.code} warn>
          {issue.code === "NO_COMPATIBLE_VERSION" && issue.unavailableLines.length > 0
            ? t("projects.issue.NO_COMPATIBLE_VERSION_LINES", {
                name,
                lines: issue.unavailableLines.join(", "),
              })
            : t(`projects.issue.${issue.code}`, { name })}
        </Detail>
      ))}
      {others.length > 0 && (
        <Detail>{t("projects.check.installedAlso", { versions: others.join(", ") })}</Detail>
      )}
      {requirement && requirement.sources.length > 0 && (
        <details className="text-[13px] text-muted-foreground">
          <summary className="w-fit cursor-pointer rounded-sm hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none">
            {t("projects.check.sources")}
          </summary>
          <ul className="mt-1 space-y-0.5 pl-4">
            {requirement.sources.map((source) => (
              <li
                key={`${source.kind}:${source.file}:${source.path}:${source.raw}`}
                className="font-mono text-xs break-words"
              >
                {source.path
                  ? t("projects.check.sourceLine", {
                      file: source.file,
                      path: source.path,
                      raw: source.raw,
                    })
                  : t("projects.check.sourceLineNoPath", { file: source.file, raw: source.raw })}
              </li>
            ))}
          </ul>
        </details>
      )}
    </ChecklistRow>
  );
}

const AVAILABLE_AS_IS: ReadonlySet<Availability> = new Set(["enabled", "unknown"]);

/** The PHP extensions the project asks for, judged by the PHP this computer would run it with. */
export function ExtensionsRow({ extensions }: { extensions: readonly ExtensionStatus[] }) {
  const { t } = useTranslation();
  if (extensions.length === 0) {
    return <ChecklistRow tone="info" title={t("projects.extensions.none")} />;
  }
  const missing = extensions.filter((e) => !AVAILABLE_AS_IS.has(e.availability));
  const title =
    missing.length > 0
      ? t("projects.extensions.missing", { count: missing.length })
      : t("projects.extensions.allReady", { count: extensions.length });
  const shown = missing.length > 0 ? missing : [];
  return (
    <ChecklistRow
      tone={missing.length > 0 ? "action" : "ok"}
      title={`${t("projects.extensions.title")}: ${title}`}
    >
      {shown.length > 0 && (
        <ul className="space-y-0.5">
          {shown.map((extension) => (
            <li key={extension.name} className="text-[13px] text-muted-foreground">
              <span className="font-mono text-xs text-foreground">{extension.name}</span>
              {": "}
              {t(`projects.availability.${extension.availability}`)}
              {extension.neededBy.length > 0 &&
                `. ${t("projects.extensions.neededBy", { who: extension.neededBy.join(", ") })}`}
            </li>
          ))}
        </ul>
      )}
      {missing.length > 0 && <Detail>{t("projects.extensions.hint")}</Detail>}
    </ChecklistRow>
  );
}

/** The database the `.env` file expects and what lunaRun suggests for it. */
export function DatabaseRow({
  suggestion,
  hint,
}: {
  suggestion: DatabaseSuggestion;
  hint: DbHint | null;
}) {
  const { t } = useTranslation();
  const engineName = t(`projects.database.engine.${suggestion.engine}`);
  let tone: Tone = "info";
  let title: string = engineName;
  if (suggestion.runtime && suggestion.line) {
    const name = `${runtimeName(suggestion.runtime)} ${suggestion.line}`;
    if (suggestion.installedVersion) {
      tone = "ok";
      title = t("projects.database.installed", {
        name,
        version: suggestion.installedVersion,
      });
    } else {
      tone = "action";
      title = t("projects.database.notInstalled", { name });
    }
  } else if (
    suggestion.note === "postgresNotSupported" ||
    suggestion.note === "sqlServerNotSupported"
  ) {
    tone = "action";
  }
  return (
    <ChecklistRow
      tone={tone}
      title={`${t("projects.database.title")}: ${title}`}
      action={
        tone === "action" && suggestion.runtime ? (
          <Link
            to="/runtimes"
            className="text-[13px] font-medium text-primary underline focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
          >
            {t("nav.runtimes")}
          </Link>
        ) : undefined
      }
    >
      {suggestion.note !== "none" && (
        <Detail>{t(`projects.database.note.${suggestion.note}`)}</Detail>
      )}
      {hint?.port != null && <Detail>{t("projects.database.port", { port: hint.port })}</Detail>}
      {hint && <Detail>{t("projects.database.from", { file: hint.file })}</Detail>}
    </ChecklistRow>
  );
}

/** What the scan could not read or had to decide, each in a sentence, with the technical detail underneath. */
export function WarningsList({ warnings }: { warnings: readonly Warning[] }) {
  const { t } = useTranslation();
  return (
    <ul className="space-y-3">
      {warnings.map((warning, index) => (
        <li key={`${warning.code}:${String(index)}`} className="flex items-start gap-3">
          <TriangleAlert className="mt-0.5 size-5 shrink-0 text-warning" aria-hidden />
          <div className="min-w-0 space-y-1">
            <p className="text-sm">
              {t(`projects.warning.${warning.code}`, { file: warning.file ?? "" })}
            </p>
            {warning.details.length > 0 && (
              <p className="font-mono text-xs break-words text-muted-foreground">
                {warning.details.join(" · ")}
              </p>
            )}
          </div>
        </li>
      ))}
    </ul>
  );
}
