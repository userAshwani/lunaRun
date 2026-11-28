import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiagnosticsView } from "@/bindings/DiagnosticsView";
import { createQueryClient } from "@/lib/query-client";
import {
  backendError,
  callsTo,
  invokeMock,
  isTauriMock,
  resetBackend,
  respondTo,
} from "@/test/backend";
import { DiagnosticsPage } from "./diagnostics-page";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const GB = 1024 * 1024 * 1024;

function view(extra: Partial<DiagnosticsView["report"]> = {}): DiagnosticsView {
  return {
    report: {
      app: {
        appVersion: "0.0.0",
        os: "windows",
        arch: "x86_64",
        lunaHome: "D:\\DIGITALLY BIRD\\lüna home",
        lunaHomeFromEnv: false,
      },
      homeSource: "default",
      freeBytes: 40 * GB,
      vcRedist: "ok",
      installed: ["mysql 8.4.13", "php 8.6.0"],
      projects: 2,
      ports: [
        { port: 80, assignedTo: null, held: true, byLunarun: false, owner: "httpd.exe" },
        { port: 443, assignedTo: null, held: false, byLunarun: false, owner: null },
        { port: 3307, assignedTo: "mysql@8.4", held: true, byLunarun: true, owner: "mysqld.exe" },
      ],
      services: [
        { id: "mysql@8.4", state: "running", error: null },
        { id: "caddy", state: "error", error: "PORT_IN_USE: port 8101 is used by httpd.exe" },
      ],
      failedJobs: [{ label: "Install PHP 8.6.0", error: "NETWORK: connection reset" }],
      registry: { source: "cache", ageSeconds: 7200, problem: null, customHost: null },
      changedSettings: ["default PHP line = 8.4"],
      ...extra,
    },
    text: "## lunaRun diagnostics\n\n- Version: 0.0.0\n- Data folder: ~\\lunaRun (default)\n",
  };
}

function renderPage() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <DiagnosticsPage />
    </QueryClientProvider>,
  );
}

function backendWith(answer: () => unknown = () => view()) {
  respondTo({ diagnostics_get: answer, jobs_list: () => [] });
}

beforeEach(() => {
  resetBackend();
});

describe("DiagnosticsPage", () => {
  it("shows a loading state first, then the system details", async () => {
    backendWith();
    renderPage();
    expect(screen.getByRole("status", { name: "Loading system information" })).toBeInTheDocument();

    expect(await screen.findByText("Windows")).toBeInTheDocument();
    expect(screen.getByText("0.0.0")).toBeInTheDocument();
    expect(screen.getByText("x86-64 (Intel / AMD)")).toBeInTheDocument();
    expect(screen.getByText("D:\\DIGITALLY BIRD\\lüna home")).toBeInTheDocument();
    expect(screen.getByText("Default location")).toBeInTheDocument();
    expect(screen.getByText("40.0 GB")).toBeInTheDocument();
    expect(screen.getByText("Installed")).toBeInTheDocument();
    expect(screen.getByText("mysql 8.4.13, php 8.6.0")).toBeInTheDocument();
    expect(screen.getByText("default PHP line = 8.4")).toBeInTheDocument();
    expect(screen.getByText(/leaves out your user name, passwords and tokens/)).toBeInTheDocument();
  });

  it("says when the data folder was set through LUNA_HOME or chosen at setup", async () => {
    backendWith(() => view({ homeSource: "env" }));
    renderPage();
    expect(await screen.findByText("Set by LUNA_HOME")).toBeInTheDocument();
  });

  it("lists the ports with who holds them", async () => {
    backendWith();
    renderPage();
    const table = await screen.findByRole("table");
    const rows = within(table).getAllByRole("row");
    expect(rows[1]).toHaveTextContent("80");
    expect(rows[1]).toHaveTextContent("Often used by other programs");
    expect(rows[1]).toHaveTextContent("Taken by httpd.exe");
    expect(rows[2]).toHaveTextContent("Free");
    expect(rows[3]).toHaveTextContent("mysql@8.4");
    expect(rows[3]).toHaveTextContent("Used by lunaRun");
  });

  it("shows the services that run or failed, and the tasks that failed", async () => {
    backendWith();
    renderPage();
    expect(await screen.findByText("caddy")).toBeInTheDocument();
    expect(screen.getByText("Failed to start")).toBeInTheDocument();
    expect(screen.getByText("PORT_IN_USE: port 8101 is used by httpd.exe")).toBeInTheDocument();
    expect(screen.getByText("Install PHP 8.6.0")).toBeInTheDocument();
    expect(screen.getByText("NETWORK: connection reset")).toBeInTheDocument();
    expect(screen.getByText(/Saved copy · 120 min old/)).toBeInTheDocument();
  });

  it("says plainly when nothing runs and nothing went wrong", async () => {
    backendWith(() => view({ services: [], failedJobs: [] }));
    renderPage();
    expect(
      await screen.findByText(/Nothing is running and nothing has gone wrong/),
    ).toBeInTheDocument();
  });

  it("does not mention Visual C++ where it is not needed", async () => {
    backendWith(() => view({ vcRedist: "notNeeded" }));
    renderPage();
    await screen.findByText("Windows");
    expect(screen.queryByText("Microsoft Visual C++ runtime")).not.toBeInTheDocument();
  });

  it("shows what failed, the category and the technical detail, and retries on request", async () => {
    let fail = true;
    backendWith(() => (fail ? backendError("PERMISSION_DENIED", "C:\\x: access denied") : view()));
    const user = userEvent.setup();
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not load system information");
    expect(alert).toHaveTextContent("Permission denied");
    expect(alert).toHaveTextContent("C:\\x: access denied");

    fail = false;
    await user.click(screen.getByRole("button", { name: /try again/i }));
    expect(await screen.findByText("Windows")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("checks again on request", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();
    await user.click(await screen.findByRole("button", { name: "Check again" }));
    await waitFor(() => {
      expect(callsTo("diagnostics_get")).toHaveLength(2);
    });
  });

  it("explains the missing backend when opened in a plain browser", async () => {
    isTauriMock.mockReturnValue(false);
    renderPage();
    await waitFor(() => {
      expect(screen.getAllByRole("alert")).toHaveLength(2);
    });
    for (const alert of screen.getAllByRole("alert")) {
      expect(alert).toHaveTextContent(/desktop app/);
    }
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("copies exactly the report the backend built and scrubbed", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Copy report" }));
    expect(await navigator.clipboard.readText()).toBe(view().text);
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("reports a failed copy instead of pretending it worked", async () => {
    backendWith();
    const user = userEvent.setup();
    renderPage();
    const button = await screen.findByRole("button", { name: "Copy report" });
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(new Error("denied"));

    await user.click(button);
    expect(await screen.findByRole("button", { name: "Copy failed" })).toBeInTheDocument();
  });
});
