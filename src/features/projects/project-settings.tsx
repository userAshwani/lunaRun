import { LoaderCircle } from "lucide-react";
import { useId, useState, type SyntheticEvent } from "react";
import { useTranslation } from "react-i18next";
import type { Binding } from "@/bindings/Binding";
import type { InstalledView } from "@/bindings/InstalledView";
import type { LunaError } from "@/bindings/LunaError";
import type { Project } from "@/bindings/Project";
import type { ProjectDetection } from "@/bindings/ProjectDetection";
import type { ProjectPatch } from "@/bindings/ProjectPatch";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import { useToast } from "@/components/toast-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/select";
import { useInstalledRuntimes, useRuntimesAvailable } from "@/features/runtimes/use-runtimes";
import { useErrorCategory } from "@/lib/errors";
import { IpcError, toLunaError } from "@/lib/ipc";
import { runtimeName } from "./project-format";
import { useUpdateProject } from "./use-projects";

interface Option {
  value: string;
  label: string;
}

const AUTOMATIC = "";

const toValue = (binding: Binding) => `${binding.mode}:${binding.value}`;

/** What can be chosen for a runtime: automatic, the lines that can be installed, and each version that is installed. */
function bindingOptions(
  runtime: string,
  catalogs: readonly RuntimeCatalog[] | undefined,
  installed: readonly InstalledView[] | undefined,
  current: Binding | null,
  labels: { line: (line: string) => string; exact: (version: string) => string },
): Option[] {
  const options = new Map<string, string>();
  const rows = catalogs?.find((catalog) => catalog.id === runtime)?.versions ?? [];
  for (const row of rows) {
    if (!row.available || row.channel !== "stable") continue;
    options.set(`line:${row.line}`, labels.line(row.line));
  }
  for (const entry of installed ?? []) {
    if (entry.runtime === runtime)
      options.set(`exact:${entry.version}`, labels.exact(entry.version));
  }
  // A choice made earlier stays visible even when it is no longer offered, so the select tells the truth.
  if (current && !options.has(toValue(current))) {
    options.set(
      toValue(current),
      current.mode === "line" ? labels.line(current.value) : labels.exact(current.value),
    );
  }
  return [...options].map(([value, label]) => ({ value, label }));
}

function parseValue(value: string): Binding {
  const at = value.indexOf(":");
  return { mode: value.slice(0, at) === "exact" ? "exact" : "line", value: value.slice(at + 1) };
}

function errorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

interface ProjectSettingsProps {
  project: Project;
  detection: ProjectDetection | undefined;
}

/** What the user can decide about a project: its name, the folder that is served, and which PHP and Node to use. */
export function ProjectSettings({ project, detection }: ProjectSettingsProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const categoryOf = useErrorCategory();
  const update = useUpdateProject();
  const catalogs = useRuntimesAvailable().data?.runtimes;
  const installed = useInstalledRuntimes().data;
  const [problem, setProblem] = useState<LunaError | null>(null);
  const [name, setName] = useState(project.name);
  const [docRoot, setDocRoot] = useState(project.docRoot);
  const ids = {
    name: useId(),
    docRoot: useId(),
    docRootHint: useId(),
    php: useId(),
    node: useId(),
  };

  const save = (patch: ProjectPatch, saved: string) => {
    setProblem(null);
    update.mutate(
      { id: project.id, patch },
      {
        onSuccess: () => {
          toast.show({ title: saved });
        },
        onError: (thrown) => {
          setProblem(errorOf(thrown));
        },
      },
    );
  };

  const labels = (runtime: string) => ({
    line: (line: string) => t("projects.settings.lineOption", { name: runtimeName(runtime), line }),
    exact: (version: string) =>
      t("projects.settings.exactOption", { name: runtimeName(runtime), version }),
  });

  const showNode = detection?.report.node != null || project.bindings.node !== null;
  const runtimes = [
    { runtime: "php", id: ids.php, current: project.bindings.php, key: "php" as const },
    ...(showNode
      ? [{ runtime: "node", id: ids.node, current: project.bindings.node, key: "node" as const }]
      : []),
  ];

  const submitName = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    save({ name }, t("projects.settings.nameSaved", { name: name.trim() }));
  };
  const submitDocRoot = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    save(
      { docRoot: { kind: "use", path: docRoot } },
      t("projects.settings.docRootSaved", { path: docRoot.trim() || "." }),
    );
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("projects.settings.title")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        {problem && (
          <div
            role="alert"
            className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2"
          >
            <p className="text-sm font-medium">{t("projects.settings.saveFailed")}</p>
            <p className="text-[13px] text-muted-foreground">{categoryOf(problem.code)}</p>
            <p className="mt-1 font-mono text-xs break-words text-muted-foreground">
              {problem.message}
            </p>
          </div>
        )}

        <form onSubmit={submitName} className="space-y-2">
          <label htmlFor={ids.name} className="text-sm font-medium">
            {t("projects.settings.name")}
          </label>
          <div className="flex max-w-xl gap-2">
            <Input
              id={ids.name}
              value={name}
              maxLength={100}
              onChange={(event) => {
                setName(event.target.value);
              }}
            />
            <Button
              type="submit"
              variant="outline"
              disabled={update.isPending || name.trim() === "" || name.trim() === project.name}
            >
              {t("projects.settings.nameSave")}
            </Button>
          </div>
        </form>

        <form onSubmit={submitDocRoot} className="space-y-2">
          <label htmlFor={ids.docRoot} className="text-sm font-medium">
            {t("projects.settings.docRoot")}
          </label>
          <div className="flex max-w-xl gap-2">
            <Input
              id={ids.docRoot}
              value={docRoot}
              className="font-mono"
              aria-describedby={ids.docRootHint}
              onChange={(event) => {
                setDocRoot(event.target.value);
              }}
            />
            <Button
              type="submit"
              variant="outline"
              disabled={update.isPending || docRoot.trim() === project.docRoot}
              aria-label={t("projects.settings.docRootSaveLabel")}
            >
              {t("projects.settings.docRootSave")}
            </Button>
          </div>
          <p id={ids.docRootHint} className="text-[13px] text-muted-foreground">
            {t("projects.settings.docRootHint")}{" "}
            {project.docRootCustom
              ? t("projects.settings.docRootCustom")
              : t("projects.settings.docRootDetected")}
          </p>
          {project.docRootCustom && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={update.isPending}
              onClick={() => {
                setDocRoot(detection?.report.docRoot.path ?? ".");
                save({ docRoot: { kind: "auto" } }, t("projects.settings.docRootAuto"));
              }}
            >
              {t("projects.settings.docRootAuto")}
            </Button>
          )}
        </form>

        {runtimes.map(({ runtime, id, current, key }) => {
          const options = bindingOptions(runtime, catalogs, installed, current, labels(runtime));
          const resolution =
            runtime === "php" ? detection?.resolution.php : detection?.resolution.node;
          return (
            <div key={runtime} className="space-y-2">
              <label htmlFor={id} className="text-sm font-medium">
                {t(runtime === "php" ? "projects.settings.php" : "projects.settings.node")}
              </label>
              <div className="flex max-w-xl items-center gap-2">
                <NativeSelect
                  id={id}
                  value={current ? toValue(current) : AUTOMATIC}
                  disabled={update.isPending}
                  onChange={(event) => {
                    const value = event.target.value;
                    const patch: ProjectPatch =
                      value === AUTOMATIC
                        ? { [key]: { kind: "auto" } }
                        : { [key]: { kind: "use", binding: parseValue(value) } };
                    save(
                      patch,
                      t("projects.settings.bindingSaved", {
                        name: runtimeName(runtime),
                        choice:
                          value === AUTOMATIC
                            ? t("projects.settings.bindingAutomatic")
                            : (options.find((option) => option.value === value)?.label ?? value),
                      }),
                    );
                  }}
                >
                  <option value={AUTOMATIC}>{t("projects.settings.automatic")}</option>
                  {options.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </NativeSelect>
                {update.isPending && (
                  <LoaderCircle
                    className="size-4 shrink-0 animate-spin motion-reduce:animate-none"
                    aria-hidden
                  />
                )}
              </div>
              {resolution?.conflictsWith !== undefined && (
                <p className="text-[13px] text-warning-soft-foreground">
                  {t("projects.check.conflict", { constraint: resolution.conflictsWith })}
                </p>
              )}
            </div>
          );
        })}
        <p className="text-[13px] text-muted-foreground">{t("projects.settings.bindingHint")}</p>
      </CardContent>
    </Card>
  );
}
