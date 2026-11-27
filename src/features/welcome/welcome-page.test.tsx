import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RouterProvider, createMemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OnboardingView } from "@/bindings/OnboardingView";
import { routes } from "@/app/router";
import { ThemeProvider } from "@/app/theme-provider";
import { ToastProvider } from "@/components/toast";
import { pickFolder } from "@/lib/native";
import { createQueryClient } from "@/lib/query-client";
import { backendError, callsTo, resetBackend, respondTo } from "@/test/backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/lib/native", () => ({
  pickFolder: vi.fn(),
  listenForFolderDrops: vi.fn(() => Promise.resolve(() => undefined)),
}));
const pickFolderMock = vi.mocked(pickFolder);

const GB = 1024 * 1024 * 1024;
const MB = 1024 * 1024;

function view(extra: Partial<OnboardingView> = {}): OnboardingView {
  return {
    show: true,
    home: "C:\\Users\\José\\AppData\\Local\\lunaRun",
    homeSource: "default",
    canChooseHome: true,
    checks: { vcRedist: "ok", freeBytes: 40 * GB, enoughSpace: true },
    pack: {
      items: [
        { runtime: "php", version: "8.6.0", line: "8.6", size: 36 * MB, installedVersion: null },
        {
          runtime: "composer",
          version: "2.11.3",
          line: "2.11",
          size: 3 * MB,
          installedVersion: null,
        },
        { runtime: "node", version: "24.12.0", line: "24", size: 32 * MB, installedVersion: null },
        {
          runtime: "mysql",
          version: "8.4.13",
          line: "8.4",
          size: 290 * MB,
          installedVersion: null,
        },
      ],
      unavailable: [],
    },
    ...extra,
  };
}

function backend(handlers: Record<string, (args?: unknown) => unknown>) {
  respondTo({
    onboarding_get: () => view(),
    onboarding_complete: () => null,
    jobs_list: () => [],
    app_info: () => ({
      appVersion: "0.0.2",
      os: "windows",
      arch: "x86_64",
      lunaHome: "C:\\x",
      lunaHomeFromEnv: false,
    }),
    ...handlers,
  });
}

function renderAt(path: string) {
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <ThemeProvider>
        <ToastProvider>
          <RouterProvider router={router} />
        </ToastProvider>
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return router;
}

/** Open the wizard and wait until what it shows has arrived (the heading is there before it). */
async function openWizard() {
  renderAt("/welcome");
  await screen.findByRole("heading", { name: "Welcome to lunaRun", level: 1 });
  return screen.findByRole("button", { name: "Continue" });
}

beforeEach(() => {
  resetBackend();
  localStorage.clear();
});

describe("the first-run wizard", () => {
  it("opens instead of the dashboard on the first run, and not afterwards", async () => {
    backend({});
    const router = renderAt("/");
    expect(await screen.findByRole("heading", { name: "Welcome to lunaRun" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/welcome");
  });

  it("does not open when it is not the first run", async () => {
    backend({ onboarding_get: () => view({ show: false, canChooseHome: false }) });
    const router = renderAt("/");
    await waitFor(() => {
      expect(callsTo("onboarding_get")).toHaveLength(1);
    });
    expect(router.state.location.pathname).toBe("/");
    expect(screen.queryByRole("heading", { name: "Welcome to lunaRun" })).not.toBeInTheDocument();
  });

  it("shows a skeleton while it loads and a way to try again when it fails", async () => {
    const user = userEvent.setup();
    let fail = true;
    backend({
      onboarding_get: () =>
        fail ? backendError("IO", "config/onboarding.json: access is denied") : view(),
    });
    renderAt("/welcome");
    expect(screen.getByRole("status", { name: "Preparing the setup" })).toBeInTheDocument();
    expect(await screen.findByText("The setup could not be prepared")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: /try again/i }));
    expect(await screen.findByText(/José/)).toBeInTheDocument();
  });

  it("shows the data folder and where it came from, then steps through the checks to the pack", async () => {
    const user = userEvent.setup();
    backend({});
    await openWizard();
    expect(screen.getByText("C:\\Users\\José\\AppData\\Local\\lunaRun")).toBeInTheDocument();
    expect(screen.getByText("The usual place")).toBeInTheDocument();
    const steps = screen.getByRole("list", { name: "Setup steps" });
    expect(within(steps).getByText("Data folder").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
    expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByText("The Microsoft Visual C++ runtime is installed")).toBeInTheDocument();
    expect(screen.getByText("40.0 GB free where the data folder is")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByRole("heading", { name: /Laravel starter pack/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install 4 runtimes" })).toBeInTheDocument();
  });

  describe("the data folder", () => {
    it("checks a chosen folder, shows what to know about it, then remembers it and restarts", async () => {
      const user = userEvent.setup();
      pickFolderMock.mockResolvedValue("D:\\OneDrive\\lunaRun");
      backend({
        onboarding_check_home: () => ({ path: "D:\\OneDrive\\lunaRun", warnings: ["synced"] }),
        onboarding_choose_home: () => ({ path: "D:\\OneDrive\\lunaRun", warnings: ["synced"] }),
        app_restart: () => null,
      });
      await openWizard();

      await user.click(screen.getByRole("button", { name: "Choose another folder" }));

      const confirm = await screen.findByRole("group", { name: "Use this folder?" });
      expect(confirm).toHaveTextContent(
        "lunaRun will restart and keep everything in D:\\OneDrive\\lunaRun",
      );
      expect(confirm).toHaveTextContent("synchronised by a cloud service");
      expect(
        callsTo("onboarding_choose_home"),
        "nothing is written before the confirmation",
      ).toEqual([]);

      await user.click(
        within(confirm).getByRole("button", { name: "Use this folder and restart" }),
      );

      await waitFor(() => {
        expect(callsTo("app_restart")).toHaveLength(1);
      });
      expect(callsTo("onboarding_choose_home")).toEqual([[{ path: "D:\\OneDrive\\lunaRun" }]]);
    });

    it("says plainly why a folder with other files cannot be used", async () => {
      const user = userEvent.setup();
      pickFolderMock.mockResolvedValue("D:\\Work");
      backend({
        onboarding_check_home: () =>
          backendError("INVALID_INPUT", "D:\\Work already has other files in it", {
            field: "home",
            reason: "notEmpty",
          }),
      });
      await openWizard();
      await user.click(screen.getByRole("button", { name: "Choose another folder" }));

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("That folder cannot be used");
      expect(alert).toHaveTextContent("It already has other files in it");
      expect(callsTo("onboarding_choose_home")).toEqual([]);
    });

    it("cancelling the picker or the confirmation changes nothing", async () => {
      const user = userEvent.setup();
      pickFolderMock.mockResolvedValueOnce(null).mockResolvedValueOnce("D:\\lunaRun");
      backend({ onboarding_check_home: () => ({ path: "D:\\lunaRun", warnings: [] }) });
      await openWizard();
      await user.click(screen.getByRole("button", { name: "Choose another folder" }));
      expect(callsTo("onboarding_check_home")).toEqual([]);

      await user.click(screen.getByRole("button", { name: "Choose another folder" }));
      const confirm = await screen.findByRole("group", { name: "Use this folder?" });
      await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("group", { name: "Use this folder?" })).not.toBeInTheDocument();
      expect(callsTo("onboarding_choose_home")).toEqual([]);
    });

    it("cannot be changed when the environment sets it", async () => {
      backend({
        onboarding_get: () => view({ homeSource: "env", canChooseHome: false }),
      });
      await openWizard();
      expect(screen.getByText("Set by the LUNA_HOME environment variable")).toBeInTheDocument();
      expect(screen.getByText(/can only be changed there/)).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Choose another folder" }),
      ).not.toBeInTheDocument();
    });
  });

  describe("the checks", () => {
    it("says what is missing and how to get it", async () => {
      const user = userEvent.setup();
      backend({
        onboarding_get: () =>
          view({ checks: { vcRedist: "missing", freeBytes: GB / 2, enoughSpace: false } }),
      });
      await openWizard();
      await user.click(screen.getByRole("button", { name: "Continue" }));

      expect(
        screen.getByText("The Microsoft Visual C++ runtime is not installed"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("https://aka.ms/vs/17/release/vc_redist.x64.exe"),
      ).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Copy the download address" })).toBeInTheDocument();
      expect(screen.getByText("Only 512 MB free where the data folder is")).toBeInTheDocument();
      expect(screen.getByText(/downloads about 361 MB/)).toBeInTheDocument();
    });

    it("does not mention Visual C++ where it is not needed", async () => {
      const user = userEvent.setup();
      backend({
        onboarding_get: () =>
          view({ checks: { vcRedist: "notNeeded", freeBytes: null, enoughSpace: true } }),
      });
      await openWizard();
      await user.click(screen.getByRole("button", { name: "Continue" }));
      expect(screen.queryByText(/Visual C\+\+/)).not.toBeInTheDocument();
      expect(screen.getByText("The free space could not be read")).toBeInTheDocument();
    });
  });

  describe("the starter pack", () => {
    async function openPack() {
      const user = userEvent.setup();
      await openWizard();
      await user.click(screen.getByRole("button", { name: "Continue" }));
      await user.click(screen.getByRole("button", { name: "Continue" }));
      return user;
    }

    it("installs what is chosen, as ordinary install jobs, and shows how each goes", async () => {
      let jobs: unknown[] = [];
      backend({
        onboarding_get: () =>
          view({
            pack: {
              items: [
                ...view().pack.items.slice(0, 3),
                {
                  runtime: "mysql",
                  version: "8.4.13",
                  line: "8.4",
                  size: 290 * MB,
                  installedVersion: "8.4.12",
                },
              ],
              unavailable: [],
            },
          }),
        runtime_install: (args) => {
          const { runtime, version } = args as { runtime: string; version: string };
          jobs = [
            ...jobs,
            {
              id: `job-${runtime}`,
              key: `${runtime}@${version}`,
              label: `Install ${runtime} ${version}`,
              state: runtime === "php" ? "running" : "succeeded",
              phase: "downloading",
              bytesDone: 18 * MB,
              bytesTotal: 36 * MB,
              message: null,
              error: null,
            },
          ];
          return { id: `job-${runtime}`, alreadyRunning: false };
        },
        jobs_list: () => jobs,
      });
      const user = await openPack();

      // MySQL 8.4 is installed already: shown, not offered again.
      expect(screen.getByText("Installed (8.4.12)")).toBeInTheDocument();
      expect(screen.getByRole("checkbox", { name: "MySQL 8.4.13" })).toBeDisabled();
      // Node.js is left out.
      await user.click(screen.getByRole("checkbox", { name: "Node.js 24.12.0" }));
      expect(screen.getByText(/Downloads about 39.0 MB/)).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Install 2 runtimes" }));

      await waitFor(() => {
        expect(callsTo("runtime_install")).toEqual([
          [{ runtime: "php", version: "8.6.0" }],
          [{ runtime: "composer", version: "2.11.3" }],
        ]);
      });
      expect(await screen.findByRole("progressbar", { name: "PHP 8.6.0" })).toBeInTheDocument();
      expect(await screen.findByText("Installed")).toBeInTheDocument();
      expect(screen.getByText(/keep running in the background/)).toBeInTheDocument();
    });

    it("finishing marks the wizard done and opens the dashboard", async () => {
      backend({});
      const router = renderAt("/welcome");
      const user = userEvent.setup();
      await user.click(await screen.findByRole("button", { name: "Continue" }));
      await user.click(screen.getByRole("button", { name: "Continue" }));
      for (const name of ["PHP 8.6.0", "Composer 2.11.3", "Node.js 24.12.0", "MySQL 8.4.13"]) {
        await user.click(screen.getByRole("checkbox", { name }));
      }
      expect(screen.getByText("Everything is installed already")).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Go to the dashboard" }));

      await waitFor(() => {
        expect(callsTo("onboarding_complete")).toHaveLength(1);
      });
      await waitFor(() => {
        expect(router.state.location.pathname).toBe("/");
      });
    });

    it("names what has no download for this computer", async () => {
      backend({
        onboarding_get: () =>
          view({ pack: { items: view().pack.items.slice(0, 3), unavailable: ["mysql"] } }),
      });
      await openPack();
      expect(screen.getByText("MySQL has no download for this computer.")).toBeInTheDocument();
    });
  });

  it("can be skipped from any step, and is then not shown again", async () => {
    const user = userEvent.setup();
    backend({});
    const router = renderAt("/welcome");
    await screen.findByRole("heading", { name: "Welcome to lunaRun" });
    await user.click(screen.getByRole("button", { name: "Skip setup" }));
    await waitFor(() => {
      expect(callsTo("onboarding_complete")).toHaveLength(1);
    });
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/");
    });
  });
});
