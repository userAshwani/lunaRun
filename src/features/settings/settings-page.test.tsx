import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Settings } from "@/bindings/Settings";
import type { SettingsView } from "@/bindings/SettingsView";
import { ThemeProvider } from "@/app/theme-provider";
import { ToastProvider } from "@/components/toast";
import { createQueryClient } from "@/lib/query-client";
import { backendError, callsTo, resetBackend, respondTo } from "@/test/backend";
import { available, fresh } from "@/test/runtimes-fixtures";
import { SettingsPage } from "./settings-page";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const DEFAULTS: Settings = {
  defaultPhpLine: null,
  defaultNodeLine: null,
  ports: {
    site: { start: 8100, end: 8999 },
    phpFastcgi: { start: 9100, end: 9999 },
    database: { start: 3306, end: 3399 },
    caddyAdmin: 2119,
  },
  registryUrl: null,
};

function view(current: Partial<Settings> = {}): SettingsView {
  return {
    current: { ...DEFAULTS, ...current },
    defaults: DEFAULTS,
    home: "C:\\Users\\José\\AppData\\Local\\lunaRun",
    homeSource: "default",
    defaultRegistryUrl:
      "https://raw.githubusercontent.com/userAshwani/lunaRun/main/registry/registry.json",
  };
}

function backend(handlers: Record<string, (args?: unknown) => unknown> = {}) {
  respondTo({
    settings_get: () => view(),
    settings_set: (args) => ({ ...view(), current: (args as { settings: Settings }).settings }),
    launch_at_login_get: () => false,
    launch_at_login_set: (args) => (args as { enabled: boolean }).enabled,
    runtimes_available: () => available(fresh),
    ...handlers,
  });
}

function renderPage() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <ThemeProvider>
        <ToastProvider>
          <MemoryRouter>
            <SettingsPage />
          </MemoryRouter>
        </ToastProvider>
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

async function loaded() {
  renderPage();
  return screen.findByRole("button", { name: "Save changes" });
}

beforeEach(() => {
  resetBackend();
  localStorage.clear();
});

describe("SettingsPage", () => {
  it("shows a skeleton, then the settings with nothing to save yet", async () => {
    backend();
    renderPage();
    expect(screen.getByRole("status", { name: "Loading the settings" })).toBeInTheDocument();
    const save = await screen.findByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    expect(screen.getByRole("radiogroup", { name: /theme/i })).toBeInTheDocument();
    expect(screen.getByLabelText("Default PHP version")).toHaveValue("");
    expect(screen.getByText("C:\\Users\\José\\AppData\\Local\\lunaRun")).toBeInTheDocument();
  });

  it("says what went wrong and offers to try again", async () => {
    const user = userEvent.setup();
    let fail = true;
    backend({
      settings_get: () =>
        fail ? backendError("IO", "config/settings.json: access is denied") : view(),
    });
    renderPage();
    expect(await screen.findByText("The settings could not be read")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: /try again/i }));
    expect(await screen.findByRole("button", { name: "Save changes" })).toBeInTheDocument();
  });

  it("saves the default versions of new projects", async () => {
    const user = userEvent.setup();
    backend();
    await loaded();

    const php = screen.getByLabelText("Default PHP version");
    // The lines come from the version list, which arrives after the settings.
    await waitFor(() => {
      expect(within(php).getAllByRole("option").length).toBeGreaterThan(1);
    });
    const options = within(php)
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(options[0]).toBe("The newest that fits (default)");
    expect(options[1]).toMatch(/^PHP \d+\.\d+$/);
    const line = within(php).getAllByRole("option")[1]?.getAttribute("value") ?? "";
    await user.selectOptions(php, line);
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => {
      expect(callsTo("settings_set")).toHaveLength(1);
    });
    const sent = (callsTo("settings_set")[0]?.[0] as { settings: Settings }).settings;
    expect(sent.defaultPhpLine).toBe(line);
    expect(sent.ports).toEqual(DEFAULTS.ports);
    expect(await screen.findByText("Settings saved")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  });

  it("shows a refused value next to its field and keeps what was typed", async () => {
    const user = userEvent.setup();
    backend({
      settings_set: () =>
        backendError(
          "INVALID_INPUT",
          "ports.site and ports.phpFastcgi overlap: a port can only have one use",
          {
            field: "ports.site",
          },
        ),
    });
    await loaded();
    const from = screen.getByLabelText("Sites: From");
    await user.clear(from);
    await user.type(from, "9050");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Sites:");
    expect(alert).toHaveTextContent("overlap");
    expect(from).toHaveValue("9050");
    expect(from).toHaveAttribute("aria-invalid", "true");
  });

  it("says new port ranges are used after a restart, and offers it", async () => {
    const user = userEvent.setup();
    backend({
      app_restart: () => backendError("BUSY", "lunaRun cannot restart while a service runs"),
    });
    await loaded();
    const admin = screen.getByLabelText("Web server admin port");
    await user.clear(admin);
    await user.type(admin, "2219");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(
      await screen.findByText("New ranges are used after lunaRun restarts."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Restart now" }));
    expect(await screen.findByText(/cannot restart while a service runs/)).toBeInTheDocument();
  });

  it("refuses an address that is not https, with the reason", async () => {
    const user = userEvent.setup();
    backend({
      settings_set: () =>
        backendError("INVALID_URL", '"http://example.org" is not an https:// address', {
          field: "registryUrl",
        }),
    });
    await loaded();
    await user.type(screen.getByLabelText("Address of the version list"), "http://example.org");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("is not an https:// address");
  });

  it("puts everything back to the defaults, to be saved", async () => {
    const user = userEvent.setup();
    backend({
      settings_get: () =>
        view({ defaultNodeLine: "22", registryUrl: "https://mirror.example/r.json" }),
    });
    await loaded();
    expect(screen.getByLabelText("Address of the version list")).toHaveValue(
      "https://mirror.example/r.json",
    );

    await user.click(screen.getByRole("button", { name: "Back to the defaults" }));

    expect(screen.getByLabelText("Address of the version list")).toHaveValue("");
    expect(screen.getByLabelText("Default Node.js version")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
  });

  describe("launch at login", () => {
    it("is a switch that asks the system and shows what it says", async () => {
      const user = userEvent.setup();
      backend();
      await loaded();
      const toggle = await screen.findByRole("switch", { name: "Start lunaRun when I log in" });
      expect(toggle).toHaveAttribute("aria-checked", "false");

      await user.click(toggle);

      await waitFor(() => {
        expect(toggle).toHaveAttribute("aria-checked", "true");
      });
      expect(callsTo("launch_at_login_set")).toEqual([[{ enabled: true }]]);
    });

    it("says when the system refuses", async () => {
      const user = userEvent.setup();
      backend({
        launch_at_login_set: () =>
          backendError("PERMISSION_DENIED", "the system did not accept the change"),
      });
      await loaded();
      await user.click(await screen.findByRole("switch", { name: "Start lunaRun when I log in" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "The system did not accept the change",
      );
    });
  });
});
