import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AvailableVersion } from "@/bindings/AvailableVersion";
import type { InstalledView } from "@/bindings/InstalledView";
import type { RegistryStatus } from "@/bindings/RegistryStatus";
import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";
import type { RuntimesAvailable } from "@/bindings/RuntimesAvailable";
import { REGISTRY_UPDATED_EVENT } from "@/lib/events";
import { createQueryClient } from "@/lib/query-client";
import {
  backendError,
  callsTo,
  emitEvent,
  isTauriMock,
  listenerCount,
  resetBackend,
  respondTo,
} from "@/test/backend";
import { RuntimesPage } from "./runtimes-page";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

function version(runtime: string, number: string, extra: Partial<AvailableVersion> = {}) {
  const line = number.split(".").slice(0, 2).join(".");
  return {
    runtime,
    version: number,
    line,
    channel: "stable",
    eol: false,
    eolDate: null,
    securityOnly: false,
    releasedAt: null,
    community: false,
    size: 31_457_280,
    available: true,
    rosettaFallback: false,
    ...extra,
  } satisfies AvailableVersion;
}

const php: RuntimeCatalog = {
  id: "php",
  displayName: "PHP",
  license: { name: "PHP License v3.01", url: "https://www.php.net/license/" },
  versions: [
    version("php", "8.4.2", { eolDate: "2028-12-31", releasedAt: "2026-10-26" }),
    version("php", "8.3.9", {
      securityOnly: true,
      eolDate: "2027-12-31",
      community: true,
    }),
    version("php", "8.1.30", { eol: true, eolDate: "2025-12-31", size: 1_048_576 }),
    version("php", "7.4.33", { eol: true, eolDate: "2022-11-28" }),
    version("php", "9.0.0", { available: false, size: null }),
  ],
};
const node: RuntimeCatalog = {
  id: "node",
  displayName: "Node.js",
  license: null,
  versions: [version("node", "24.1.0", { rosettaFallback: true })],
};
const caddy: RuntimeCatalog = {
  id: "caddy",
  displayName: "Caddy",
  license: null,
  versions: [version("caddy", "2.9.0", { available: false, size: null })],
};

const fresh: RegistryStatus = {
  source: "fresh",
  ageSeconds: null,
  problem: null,
  refreshing: false,
};

function available(registry: RegistryStatus = fresh, runtimes = [caddy, node, php]) {
  return { runtimes, registry } satisfies RuntimesAvailable;
}

const installedPhp: InstalledView = {
  runtime: "php",
  version: "8.3.9",
  dir: "D:\\DIGITALLY BIRD\\lüna home\\runtimes\\php\\8.3.9",
  installedAt: 1_767_225_600, // 2026-01-01 UTC
  rosetta: false,
  parts: 1,
};

function renderPage() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <RuntimesPage />
    </QueryClientProvider>,
  );
}

function backendWith(
  handlers: {
    available?: () => unknown;
    installed?: () => unknown;
    refresh?: () => unknown;
  } = {},
) {
  respondTo({
    runtimes_available: handlers.available ?? (() => available()),
    runtimes_installed: handlers.installed ?? (() => [installedPhp]),
    registry_refresh: handlers.refresh ?? (() => fresh),
  });
}

beforeEach(() => {
  resetBackend();
});

describe("RuntimesPage", () => {
  it("shows a loading state first, then PHP before the alphabetical order of the backend", async () => {
    backendWith();
    renderPage();
    expect(screen.getByRole("status", { name: "Loading runtimes" })).toBeInTheDocument();

    const tabs = await screen.findAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["PHP1", "Node.js", "Caddy"]);
    expect(screen.getByRole("tab", { name: /PHP/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("status", { name: "Loading runtimes" })).not.toBeInTheDocument();
  });

  it("lists what is installed with its date and folder", async () => {
    backendWith();
    renderPage();

    const installed = await screen.findByRole("region", { name: "Installed" });
    expect(within(installed).getByText("8.3.9")).toBeInTheDocument();
    expect(within(installed).getByText(/Jan 1, 2026/)).toBeInTheDocument();
    expect(within(installed).getByText(installedPhp.dir)).toBeInTheDocument();
  });

  it("says plainly when nothing of the runtime is installed", async () => {
    backendWith({ installed: () => [] });
    renderPage();

    expect(await screen.findByText("Nothing installed")).toBeInTheDocument();
    expect(
      screen.getByText("No PHP version is installed yet. Pick one from the list below."),
    ).toBeInTheDocument();
  });

  it("marks end of life, security-only, community and installed versions, hides unavailable ones", async () => {
    backendWith();
    renderPage();

    const list = await screen.findByRole("region", { name: "Available PHP versions" });
    const row = (number: string) => within(list).getByText(number).closest("tr") as HTMLElement;

    expect(within(row("8.4.2")).getByText("Supported until Dec 31, 2028")).toBeInTheDocument();
    expect(within(row("8.4.2")).getByText("Oct 26, 2026")).toBeInTheDocument();
    expect(within(row("8.4.2")).getByText("30.0 MB")).toBeInTheDocument();

    const security = row("8.3.9");
    expect(within(security).getByText("Security fixes until Dec 31, 2027")).toBeInTheDocument();
    expect(within(security).getByText("Community build")).toBeInTheDocument();
    expect(within(security).getByText("Installed")).toBeInTheDocument();
    expect(within(row("8.4.2")).queryByText("Installed")).not.toBeInTheDocument();

    expect(within(list).queryByText("9.0.0")).not.toBeInTheDocument();
  });

  it("hides end-of-life versions until asked, and says how many there are", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();

    const list = await screen.findByRole("region", { name: "Available PHP versions" });
    expect(within(list).queryByText("8.1.30")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show end-of-life versions (2)" }));
    const row = within(list).getByText("8.1.30").closest("tr") as HTMLElement;
    expect(within(row).getByText("End of life")).toBeInTheDocument();
    expect(within(row).getByText("1.0 MB")).toBeInTheDocument();
    expect(within(list).getByText("7.4.33")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Hide end-of-life versions" }));
    expect(within(list).queryByText("8.1.30")).not.toBeInTheDocument();
  });

  it("shows the license of the runtime", async () => {
    backendWith();
    renderPage();
    expect(await screen.findByText(/License: PHP License v3.01/)).toBeInTheDocument();
    expect(screen.getByText("https://www.php.net/license/")).toBeInTheDocument();
  });

  it("switches runtime and shows the Rosetta note", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("tab", { name: "Node.js" }));
    expect(screen.getByRole("tab", { name: "Node.js" })).toHaveAttribute("aria-selected", "true");
    const list = screen.getByRole("region", { name: "Available Node.js versions" });
    expect(within(list).getByText("24.1.0")).toBeInTheDocument();
    expect(within(list).getByText("Runs under Rosetta")).toBeInTheDocument();
    expect(
      screen.getByText("No Node.js version is installed yet. Pick one from the list below."),
    ).toBeInTheDocument();
  });

  it("moves between tabs with the arrow keys", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();

    const first = await screen.findByRole("tab", { name: /PHP/ });
    first.focus();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Node.js" })).toHaveFocus();
    expect(screen.getByRole("tab", { name: "Node.js" })).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{End}");
    expect(screen.getByRole("tab", { name: "Caddy" })).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: /PHP/ })).toHaveFocus();
  });

  it("says when there is no build of a runtime for this computer", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("tab", { name: "Caddy" }));
    expect(screen.getByText("No Caddy build for this computer")).toBeInTheDocument();
  });

  it("offers the end-of-life versions when they are all there is", async () => {
    const old: RuntimeCatalog = {
      ...php,
      versions: [version("php", "5.6.40", { eol: true, eolDate: "2018-12-31" })],
    };
    backendWith({ available: () => available(fresh, [old]) });
    const user = userEvent.setup();
    renderPage();

    expect(
      await screen.findByText("Every available PHP version has reached end of life."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show end-of-life versions (1)" }));
    expect(screen.getByText("5.6.40")).toBeInTheDocument();
  });

  it("keeps a tab for an installed runtime the list no longer knows", async () => {
    backendWith({
      installed: () => [{ ...installedPhp, runtime: "legacy-tool", version: "1.0.0" }],
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("tab", { name: /legacy-tool/ }));
    const installed = screen.getByRole("region", { name: "Installed" });
    expect(within(installed).getByText("1.0.0")).toBeInTheDocument();
    expect(screen.getByText("No legacy-tool build for this computer")).toBeInTheDocument();
  });

  describe("version list status", () => {
    it("says the list is up to date", async () => {
      backendWith();
      renderPage();
      expect(await screen.findByText("The version list is up to date.")).toBeInTheDocument();
    });

    it("says offline with the age of the saved list", async () => {
      const offline: RegistryStatus = {
        source: "cache",
        ageSeconds: 3 * 3600 + 120,
        problem: { code: "NETWORK", message: "connection refused" },
        refreshing: false,
      };
      backendWith({ available: () => available(offline) });
      renderPage();
      expect(
        await screen.findByText(
          "Showing the version list saved 3 hours ago. lunaRun could not reach the version list server. You appear to be offline.",
        ),
      ).toBeInTheDocument();
    });

    it("says the built-in list is in use when nothing was ever downloaded", async () => {
      const bundled: RegistryStatus = {
        source: "bundled",
        ageSeconds: null,
        problem: { code: "NETWORK", message: "offline" },
        refreshing: false,
      };
      backendWith({ available: () => available(bundled) });
      renderPage();
      expect(
        await screen.findByText(
          /Showing the version list that came with this lunaRun\. lunaRun could not reach/,
        ),
      ).toBeInTheDocument();
    });

    it("asks to update lunaRun when the newest list needs a newer app", async () => {
      const outdated: RegistryStatus = {
        source: "bundled",
        ageSeconds: null,
        problem: { code: "REGISTRY_UNSUPPORTED_SCHEMA", message: "schema 2" },
        refreshing: false,
      };
      backendWith({ available: () => available(outdated) });
      renderPage();
      expect(
        await screen.findByText(/Update lunaRun to see the latest versions/),
      ).toBeInTheDocument();
    });

    it("shows the technical reason for any other problem", async () => {
      const broken: RegistryStatus = {
        source: "cache",
        ageSeconds: 30,
        problem: { code: "REGISTRY_INVALID", message: "unexpected token at line 3" },
        refreshing: false,
      };
      backendWith({ available: () => available(broken) });
      renderPage();
      expect(
        await screen.findByText(
          /The version list could not be refreshed: unexpected token at line 3/,
        ),
      ).toBeInTheDocument();
      expect(screen.getByText(/saved less than a minute ago/)).toBeInTheDocument();
    });

    it("shows that the background check is running and disables the button", async () => {
      backendWith({
        available: () => available({ ...fresh, source: "bundled", refreshing: true }),
      });
      renderPage();
      expect(await screen.findByText("Checking for newer versions...")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Checking..." })).toBeDisabled();
    });

    it("loads the new list when the background refresh finishes", async () => {
      let current = available({ ...fresh, source: "bundled", refreshing: true }, [node]);
      backendWith({ available: () => current });
      renderPage();
      await screen.findByText("Checking for newer versions...");
      expect(screen.queryByRole("tab", { name: /PHP/ })).not.toBeInTheDocument();

      current = available(fresh, [node, php]);
      emitEvent(REGISTRY_UPDATED_EVENT, fresh);

      expect(await screen.findByRole("tab", { name: /PHP/ })).toBeInTheDocument();
      expect(screen.getByText("The version list is up to date.")).toBeInTheDocument();
    });

    it("stops listening when the page closes", async () => {
      backendWith();
      const view = renderPage();
      await screen.findAllByRole("tab");
      await waitFor(() => {
        expect(listenerCount(REGISTRY_UPDATED_EVENT)).toBe(1);
      });
      view.unmount();
      expect(listenerCount(REGISTRY_UPDATED_EVENT)).toBe(0);
    });
  });

  describe("check for updates", () => {
    it("asks the backend and shows the result", async () => {
      let status = fresh;
      backendWith({
        available: () => available(status),
        refresh: () => {
          status = { ...fresh, source: "cache", ageSeconds: 5, problem: null };
          return status;
        },
      });
      const user = userEvent.setup();
      renderPage();

      await user.click(await screen.findByRole("button", { name: "Check for updates" }));

      expect(await screen.findByText(/saved less than a minute ago/)).toBeInTheDocument();
      expect(callsTo("registry_refresh")).toHaveLength(1);
    });

    it("reports when the check itself failed", async () => {
      backendWith({ refresh: () => backendError("INTERNAL", "refresh task crashed") });
      const user = userEvent.setup();
      renderPage();

      await user.click(await screen.findByRole("button", { name: "Check for updates" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "Could not check for updates: refresh task crashed",
      );
    });
  });

  describe("errors", () => {
    it("shows what failed and retries on request", async () => {
      let attempts = 0;
      backendWith({
        available: () => {
          attempts += 1;
          return attempts === 1 ? backendError("INTERNAL", "clock unusable") : available();
        },
      });
      const user = userEvent.setup();
      renderPage();

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Could not load the runtimes");
      expect(alert).toHaveTextContent("clock unusable");

      await user.click(screen.getByRole("button", { name: "Try again" }));
      expect(await screen.findAllByRole("tab")).toHaveLength(3);
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("keeps the list when only the installed versions cannot be read", async () => {
      let attempts = 0;
      backendWith({
        installed: () => {
          attempts += 1;
          return attempts === 1
            ? backendError("IO", "cannot read the runtimes folder")
            : [installedPhp];
        },
      });
      const user = userEvent.setup();
      renderPage();

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Could not read the installed versions");
      expect(alert).toHaveTextContent("cannot read the runtimes folder");
      expect(screen.getByRole("region", { name: "Available PHP versions" })).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Try again" }));
      expect(await screen.findByRole("region", { name: "Installed" })).toBeInTheDocument();
    });

    it("explains the missing backend when opened in a plain browser", async () => {
      isTauriMock.mockReturnValue(false);
      renderPage();
      expect(await screen.findByRole("alert")).toHaveTextContent(/desktop app/);
    });
  });
});
