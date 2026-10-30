import { QueryClientProvider } from "@tanstack/react-query";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppInfo } from "@/bindings/AppInfo";
import { createQueryClient } from "@/lib/query-client";
import { DiagnosticsPage } from "./diagnostics-page";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const isTauriMock = vi.mocked(isTauri);

const info: AppInfo = {
  appVersion: "0.0.0",
  os: "windows",
  arch: "x86_64",
  lunaHome: "D:\\DIGITALLY BIRD\\lüna home",
  lunaHomeFromEnv: false,
};

function renderPage() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <DiagnosticsPage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  isTauriMock.mockReturnValue(true);
});

describe("DiagnosticsPage", () => {
  it("shows a loading state first, then the system details", async () => {
    invokeMock.mockResolvedValue(info);
    renderPage();
    expect(screen.getByRole("status", { name: "Loading system information" })).toBeInTheDocument();

    expect(await screen.findByText("Windows")).toBeInTheDocument();
    expect(screen.getByText("0.0.0")).toBeInTheDocument();
    expect(screen.getByText("x86-64 (Intel / AMD)")).toBeInTheDocument();
    expect(screen.getByText("D:\\DIGITALLY BIRD\\lüna home")).toBeInTheDocument();
    expect(screen.getByText("Default location")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("says when the data folder was set through LUNA_HOME", async () => {
    invokeMock.mockResolvedValue({ ...info, lunaHomeFromEnv: true });
    renderPage();
    expect(await screen.findByText("Set by LUNA_HOME")).toBeInTheDocument();
  });

  it("shows what failed, the category and the technical detail, and retries on request", async () => {
    invokeMock.mockRejectedValueOnce({
      code: "PERMISSION_DENIED",
      message: "C:\\x: access denied",
    });
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not load system information");
    expect(alert).toHaveTextContent("Permission denied");
    expect(alert).toHaveTextContent("C:\\x: access denied");

    invokeMock.mockResolvedValueOnce(info);
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Windows")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("falls back to a generic category for an error code from a newer backend", async () => {
    invokeMock.mockRejectedValueOnce({ code: "SOMETHING_NEW", message: "new failure" });
    renderPage();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Unexpected error");
    expect(alert).toHaveTextContent("new failure");
  });

  it("explains the missing backend when opened in a plain browser", async () => {
    isTauriMock.mockReturnValue(false);
    renderPage();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/desktop app/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("copies the report to the clipboard and confirms", async () => {
    invokeMock.mockResolvedValue(info);
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Copy report" }));
    expect(await navigator.clipboard.readText()).toBe(
      [
        "lunaRun diagnostics",
        "Version: 0.0.0",
        "OS: windows",
        "Architecture: x86_64",
        "Data folder: D:\\DIGITALLY BIRD\\lüna home",
      ].join("\n"),
    );
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("reports a failed copy instead of pretending it worked", async () => {
    invokeMock.mockResolvedValue(info);
    const user = userEvent.setup();
    renderPage();
    const button = await screen.findByRole("button", { name: "Copy report" });
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(new Error("denied"));

    await user.click(button);
    expect(await screen.findByRole("button", { name: "Copy failed" })).toBeInTheDocument();
  });
});
