import { CircleAlert, LoaderCircle, RotateCcw, Save } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { LunaError } from "@/bindings/LunaError";
import type { PortSettings } from "@/bindings/PortSettings";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import type { Settings } from "@/bindings/Settings";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
import { PageHeader } from "@/components/page-header";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { useToast } from "@/components/toast-context";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { runtimeName } from "@/features/projects/project-format";
import { useRuntimesAvailable } from "@/features/runtimes/use-runtimes";
import { useErrorCategory } from "@/lib/errors";
import { IpcError, ipc, toLunaError } from "@/lib/ipc";
import {
  useLaunchAtLogin,
  useSaveSettings,
  useSetLaunchAtLogin,
  useSettings,
} from "./use-settings";

type Field =
  | "defaultPhpLine"
  | "defaultNodeLine"
  | "ports.site"
  | "ports.phpFastcgi"
  | "ports.database"
  | "ports.caddyAdmin"
  | "registryUrl";

function errorOf(thrown: unknown): LunaError {
  return thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
}

function fieldOf(error: LunaError | null): Field | null {
  const field = error?.details?.field;
  return typeof field === "string" ? (field as Field) : null;
}

/** The supported lines of a runtime, newest first, from the version list. */
function linesOf(catalog: readonly RuntimeCatalog[] | undefined, runtime: string): string[] {
  const versions = catalog?.find((c) => c.id === runtime)?.versions ?? [];
  const lines: string[] = [];
  for (const version of versions) {
    if (version.eol || version.channel !== "stable") continue;
    if (!lines.includes(version.line)) lines.push(version.line);
  }
  return lines;
}

function Row({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor?: string;
  children: ReactNode;
}) {
  const hintId = useId();
  return (
    <div className="grid gap-2 py-4 first:pt-0 last:pb-0 md:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)] md:gap-6">
      <div className="min-w-0">
        {htmlFor ? (
          <label htmlFor={htmlFor} className="text-sm font-medium">
            {label}
          </label>
        ) : (
          <p className="text-sm font-medium">{label}</p>
        )}
        {hint && (
          <p id={hintId} className="mt-1 text-[13px] text-muted-foreground">
            {hint}
          </p>
        )}
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function FieldError({ error, field }: { error: LunaError | null; field: Field }) {
  const { t } = useTranslation();
  if (fieldOf(error) !== field || !error) return null;
  return (
    <p
      role="alert"
      className="mt-2 flex items-start gap-2 text-[13px] text-destructive-soft-foreground"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
      <span>
        <span className="font-medium">{t(`settingsPage.fields.${field}`)}: </span>
        {error.message}
      </span>
    </p>
  );
}

function LaunchAtLogin() {
  const { t } = useTranslation();
  const login = useLaunchAtLogin();
  const setLogin = useSetLaunchAtLogin();
  const id = useId();
  const failure = setLogin.isError ? errorOf(setLogin.error) : null;
  return (
    <Row
      label={t("settingsPage.general.login")}
      hint={t("settingsPage.general.loginHint")}
      htmlFor={id}
    >
      {login.isPending ? (
        <Skeleton className="h-6 w-11 rounded-full" />
      ) : (
        <button
          id={id}
          type="button"
          role="switch"
          aria-checked={login.data === true}
          disabled={setLogin.isPending || login.isError}
          onClick={() => {
            setLogin.mutate(!(login.data === true));
          }}
          className="relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border border-transparent bg-muted-foreground/75 transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50 aria-checked:bg-primary"
        >
          <span className="sr-only">{t("settingsPage.general.login")}</span>
          <span
            aria-hidden
            className="inline-block size-5 translate-x-0.5 rounded-full bg-card shadow-xs transition-transform [[aria-checked=true]>&]:translate-x-5"
          />
        </button>
      )}
      {(failure ?? (login.isError ? errorOf(login.error) : null)) && (
        <p role="alert" className="mt-2 text-[13px] text-destructive-soft-foreground">
          {t("settingsPage.general.loginFailed")}: {(failure ?? errorOf(login.error)).message}
        </p>
      )}
    </Row>
  );
}

function PortRange({
  name,
  field,
  value,
  fallback,
  onChange,
  error,
}: {
  name: string;
  field: Field;
  value: { start: number; end: number };
  fallback: { start: number; end: number };
  onChange: (next: { start: number; end: number }) => void;
  error: LunaError | null;
}) {
  const { t } = useTranslation();
  const startId = useId();
  const invalid = fieldOf(error) === field;
  const number = (text: string) => Math.max(0, Math.min(65535, Number.parseInt(text, 10) || 0));
  return (
    <Row
      label={name}
      hint={t("settingsPage.ports.defaultHint", { start: fallback.start, end: fallback.end })}
      htmlFor={startId}
    >
      <div className="flex items-center gap-2">
        <Input
          id={startId}
          inputMode="numeric"
          aria-label={`${name}: ${t("settingsPage.ports.from")}`}
          aria-invalid={invalid}
          className="w-28 tabular-nums"
          value={String(value.start)}
          onChange={(event) => {
            onChange({ ...value, start: number(event.target.value) });
          }}
        />
        <span className="text-sm text-muted-foreground">–</span>
        <Input
          inputMode="numeric"
          aria-label={`${name}: ${t("settingsPage.ports.to")}`}
          aria-invalid={invalid}
          className="w-28 tabular-nums"
          value={String(value.end)}
          onChange={(event) => {
            onChange({ ...value, end: number(event.target.value) });
          }}
        />
      </div>
      <FieldError error={error} field={field} />
    </Row>
  );
}

function SettingsForm({
  saved,
  defaults,
  defaultRegistryUrl,
}: {
  saved: Settings;
  defaults: Settings;
  defaultRegistryUrl: string;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const categoryOf = useErrorCategory();
  const available = useRuntimesAvailable();
  const save = useSaveSettings();
  const [draft, setDraft] = useState<Settings>(saved);
  const [portsChanged, setPortsChanged] = useState(false);
  const [restartError, setRestartError] = useState<LunaError | null>(null);
  const ids = { php: useId(), node: useId(), admin: useId(), registry: useId() };

  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const error = save.isError ? errorOf(save.error) : null;
  const catalog = available.data?.runtimes;
  const phpLines = linesOf(catalog, "php");
  const nodeLines = linesOf(catalog, "node");
  const withCurrent = (lines: string[], current: string | null) =>
    current && !lines.includes(current) ? [current, ...lines] : lines;
  const ports = (patch: Partial<PortSettings>) => {
    setDraft({ ...draft, ports: { ...draft.ports, ...patch } });
  };

  const submit = () => {
    const portsMoved = JSON.stringify(draft.ports) !== JSON.stringify(saved.ports);
    save.mutate(draft, {
      onSuccess: (view) => {
        // What was stored, as the backend tidied it (spaces trimmed, an empty field back to the default).
        setDraft(view.current);
        toast.show({ title: t("settingsPage.saved"), tone: "success" });
        if (portsMoved) setPortsChanged(true);
      },
    });
  };

  return (
    <form
      className="space-y-6"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <Card>
        <CardHeader>
          <CardTitle>{t("settingsPage.general.title")}</CardTitle>
        </CardHeader>
        <CardContent className="divide-y">
          <Row label={t("settingsPage.general.theme")} hint={t("settingsPage.general.themeHint")}>
            <div className="max-w-48">
              <ThemeSwitcher />
            </div>
          </Row>
          <LaunchAtLogin />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("settingsPage.runtimes.title")}</CardTitle>
          <p className="text-sm text-muted-foreground">{t("settingsPage.runtimes.text")}</p>
        </CardHeader>
        <CardContent className="divide-y">
          {(
            [
              ["php", ids.php, "defaultPhpLine", phpLines, t("settingsPage.runtimes.php")],
              ["node", ids.node, "defaultNodeLine", nodeLines, t("settingsPage.runtimes.node")],
            ] as const
          ).map(([runtime, id, field, lines, label]) => (
            <Row key={runtime} label={label} htmlFor={id}>
              <NativeSelect
                id={id}
                aria-invalid={fieldOf(error) === field}
                value={draft[field] ?? ""}
                onChange={(event) => {
                  setDraft({ ...draft, [field]: event.target.value || null });
                }}
              >
                <option value="">{t("settingsPage.runtimes.newest")}</option>
                {withCurrent(lines, draft[field]).map((line) => (
                  <option key={line} value={line}>
                    {t("settingsPage.runtimes.line", { name: runtimeName(runtime), line })}
                  </option>
                ))}
              </NativeSelect>
              <FieldError error={error} field={field} />
            </Row>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("settingsPage.ports.title")}</CardTitle>
          <p className="text-sm text-muted-foreground">{t("settingsPage.ports.text")}</p>
        </CardHeader>
        <CardContent className="divide-y">
          <PortRange
            name={t("settingsPage.ports.site")}
            field="ports.site"
            value={draft.ports.site}
            fallback={defaults.ports.site}
            onChange={(site) => {
              ports({ site });
            }}
            error={error}
          />
          <PortRange
            name={t("settingsPage.ports.phpFastcgi")}
            field="ports.phpFastcgi"
            value={draft.ports.phpFastcgi}
            fallback={defaults.ports.phpFastcgi}
            onChange={(phpFastcgi) => {
              ports({ phpFastcgi });
            }}
            error={error}
          />
          <PortRange
            name={t("settingsPage.ports.database")}
            field="ports.database"
            value={draft.ports.database}
            fallback={defaults.ports.database}
            onChange={(database) => {
              ports({ database });
            }}
            error={error}
          />
          <Row
            label={t("settingsPage.ports.caddyAdmin")}
            hint={t("settingsPage.ports.defaultHintOne", { port: defaults.ports.caddyAdmin })}
            htmlFor={ids.admin}
          >
            <Input
              id={ids.admin}
              inputMode="numeric"
              aria-invalid={fieldOf(error) === "ports.caddyAdmin"}
              className="w-28 tabular-nums"
              value={String(draft.ports.caddyAdmin)}
              onChange={(event) => {
                ports({
                  caddyAdmin: Math.max(
                    0,
                    Math.min(65535, Number.parseInt(event.target.value, 10) || 0),
                  ),
                });
              }}
            />
            <FieldError error={error} field="ports.caddyAdmin" />
          </Row>
          {portsChanged && (
            <div className="space-y-2 pt-4" role="status">
              <p className="text-[13px] text-warning-soft-foreground">
                {t("settingsPage.ports.restart")}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  setRestartError(null);
                  ipc.appRestart().catch((thrown: unknown) => {
                    setRestartError(errorOf(thrown));
                  });
                }}
              >
                <RotateCcw aria-hidden />
                {t("settingsPage.ports.restartNow")}
              </Button>
              {restartError && (
                <p role="alert" className="text-[13px] text-destructive-soft-foreground">
                  {restartError.code === "BUSY"
                    ? t("settingsPage.ports.restartBusy")
                    : categoryOf(restartError.code)}
                </p>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("settingsPage.advanced.title")}</CardTitle>
        </CardHeader>
        <CardContent>
          <Row
            label={t("settingsPage.advanced.registry")}
            hint={t("settingsPage.advanced.registryHint")}
            htmlFor={ids.registry}
          >
            <Input
              id={ids.registry}
              type="url"
              spellCheck={false}
              placeholder={defaultRegistryUrl}
              aria-invalid={fieldOf(error) === "registryUrl"}
              value={draft.registryUrl ?? ""}
              onChange={(event) => {
                setDraft({ ...draft, registryUrl: event.target.value || null });
              }}
            />
            <FieldError error={error} field="registryUrl" />
          </Row>
        </CardContent>
      </Card>

      {error && fieldOf(error) === null && (
        <div
          role="alert"
          className="flex items-start gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2"
        >
          <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
          <div className="min-w-0">
            <p className="text-sm font-medium">{t("settingsPage.saveFailed")}</p>
            <p className="text-[13px] text-muted-foreground">{categoryOf(error.code)}</p>
            <p className="mt-1 font-mono text-xs break-words text-muted-foreground">
              {error.message}
            </p>
          </div>
        </div>
      )}

      <div className="sticky bottom-0 -mx-1 flex flex-wrap items-center justify-end gap-2 bg-background/95 px-1 py-3">
        <Button
          type="button"
          variant="outline"
          disabled={save.isPending || JSON.stringify(draft) === JSON.stringify(defaults)}
          onClick={() => {
            setDraft(defaults);
          }}
        >
          {t("settingsPage.reset")}
        </Button>
        <Button type="submit" disabled={!dirty || save.isPending}>
          {save.isPending ? (
            <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden />
          ) : (
            <Save aria-hidden />
          )}
          {save.isPending ? t("settingsPage.saving") : t("settingsPage.save")}
        </Button>
      </div>
    </form>
  );
}

/** Settings (FR-AP-03, D-045): theme, launch at login, the default versions of new projects, ports, the version list's
 * address, and the data folder (shown; it is chosen in the first-run setup). */
export function SettingsPage() {
  const { t } = useTranslation();
  const settings = useSettings();

  let body: ReactNode;
  if (settings.isPending) {
    body = (
      <div className="space-y-6" role="status" aria-label={t("settingsPage.loading")}>
        {[0, 1, 2].map((key) => (
          <Card key={key}>
            <CardContent className="space-y-3 pt-6">
              <Skeleton className="h-5 w-40" />
              <Skeleton className="h-9 w-full" />
              <Skeleton className="h-9 w-2/3" />
            </CardContent>
          </Card>
        ))}
      </div>
    );
  } else if (settings.isError) {
    body = (
      <ErrorState
        error={errorOf(settings.error)}
        title={t("settingsPage.loadError")}
        onRetry={() => {
          void settings.refetch();
        }}
      />
    );
  } else {
    const view = settings.data;
    body = (
      <div className="space-y-6">
        <SettingsForm
          saved={view.current}
          defaults={view.defaults}
          defaultRegistryUrl={view.defaultRegistryUrl}
        />
        <Card>
          <CardHeader>
            <CardTitle>{t("settingsPage.home.title")}</CardTitle>
            <p className="text-sm text-muted-foreground">{t("settingsPage.home.text")}</p>
          </CardHeader>
          <CardContent>
            <div className="flex flex-wrap items-center gap-3">
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
              <CopyButton
                text={view.home}
                label={t("settingsPage.home.copy")}
                doneLabel={t("settingsPage.home.copied")}
                failedLabel={t("settingsPage.home.copyFailed")}
                size="sm"
              />
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <>
      <PageHeader title={t("nav.settings")} description={t("pages.settings.description")} />
      {body}
    </>
  );
}
