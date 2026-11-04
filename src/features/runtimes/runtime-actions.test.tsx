import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { JobInfo } from "@/bindings/JobInfo";
import type { UsageReport } from "@/bindings/UsageReport";
import { JOB_DONE_EVENT, JOB_PROGRESS_EVENT } from "@/lib/events";
import { backendError, callsTo, emitEvent, listenerCount, resetBackend } from "@/test/backend";
import { backendWith, installedPhp, renderPage, runtimeJob } from "@/test/runtimes-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const noUsage: UsageReport = {
  runningServices: [],
  boundProjects: [],
  dataDirs: [],
  dataShared: false,
};

const MB = 1024 * 1024;

beforeEach(() => {
  resetBackend();
});

/** The row of a version in the Available table. */
async function availableRow(version: string): Promise<HTMLElement> {
  const list = await screen.findByRole("region", { name: "Available PHP versions" });
  return within(list).getByText(version).closest("tr") as HTMLElement;
}

async function liveEvents() {
  await waitFor(() => {
    expect(listenerCount(JOB_PROGRESS_EVENT)).toBe(1);
    // The task list and the page effects (reload, toast) each listen for the end of a job.
    expect(listenerCount(JOB_DONE_EVENT)).toBe(2);
  });
}

describe("installing", () => {
  it("offers Install on versions that are not installed yet, and only on those", async () => {
    backendWith();
    renderPage();

    const fresh = await availableRow("8.4.2");
    expect(within(fresh).getByRole("button", { name: "Install PHP 8.4.2" })).toBeEnabled();
    const have = await availableRow("8.3.9");
    expect(within(have).queryByRole("button", { name: /Install/ })).not.toBeInTheDocument();
    expect(within(have).getByText("Installed")).toBeInTheDocument();
  });

  it("starts the install, then shows live progress with the phase, percentage and sizes", async () => {
    let jobs: JobInfo[] = [];
    backendWith({
      jobs: () => jobs,
      more: {
        runtime_install: () => {
          jobs = [runtimeJob("php", "8.4.2", { bytesDone: 5 * MB, bytesTotal: 20 * MB })];
          return { id: jobs[0]?.id, alreadyRunning: false };
        },
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Install PHP 8.4.2" }));

    expect(callsTo("runtime_install")).toEqual([[{ runtime: "php", version: "8.4.2" }]]);
    const row = await availableRow("8.4.2");
    const bar = await within(row).findByRole("progressbar", {
      name: "Progress of the installation of PHP 8.4.2",
    });
    expect(bar).toHaveAttribute("aria-valuenow", "25");
    expect(within(row).getByText("Downloading")).toBeInTheDocument();
    expect(within(row).getByText("25%")).toBeInTheDocument();
    expect(within(row).getByText("5.0 MB of 20.0 MB")).toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: /Install PHP/ })).not.toBeInTheDocument();
  });

  it("moves the bar and the phase when progress events arrive", async () => {
    backendWith({
      jobs: () => [runtimeJob("php", "8.4.2", { bytesDone: 5 * MB, bytesTotal: 20 * MB })],
    });
    renderPage();
    const row = await availableRow("8.4.2");
    await within(row).findByRole("progressbar");
    await liveEvents();

    emitEvent(JOB_PROGRESS_EVENT, {
      jobId: "job-php-8.4.2",
      phase: "extracting",
      bytesDone: 3,
      bytesTotal: 4,
      message: null,
    });

    await waitFor(() => {
      expect(within(row).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "75");
    });
    expect(within(row).getByText("Extracting")).toBeInTheDocument();
  });

  it("cancels the running install", async () => {
    backendWith({
      jobs: () => [runtimeJob("php", "8.4.2", { bytesDone: MB, bytesTotal: 20 * MB })],
      more: { job_cancel: () => true },
    });
    const user = userEvent.setup();
    renderPage();

    const row = await availableRow("8.4.2");
    await user.click(
      await within(row).findByRole("button", { name: "Cancel the installation of PHP 8.4.2" }),
    );

    expect(callsTo("job_cancel")).toEqual([[{ jobId: "job-php-8.4.2" }]]);
  });

  it("confirms a finished install with a toast and shows the new version as installed", async () => {
    let jobs: JobInfo[] = [runtimeJob("php", "8.4.2", { label: "Install PHP 8.4.2" })];
    let installed = [installedPhp];
    backendWith({ jobs: () => jobs, installed: () => installed });
    renderPage();
    const row = await availableRow("8.4.2");
    await within(row).findByRole("progressbar");
    await liveEvents();

    jobs = [{ ...runtimeJob("php", "8.4.2"), state: "succeeded" }];
    installed = [installedPhp, { ...installedPhp, version: "8.4.2" }];
    emitEvent(JOB_DONE_EVENT, { jobId: "job-php-8.4.2", ok: true });

    expect(await screen.findByText("PHP 8.4.2 installed.")).toBeInTheDocument();
    await waitFor(() => {
      expect(within(row).getByText("Installed")).toBeInTheDocument();
    });
    expect(within(row).queryByRole("progressbar")).not.toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: /Install PHP/ })).not.toBeInTheDocument();
  });

  it("says a cancelled install kept its download", async () => {
    let jobs: JobInfo[] = [runtimeJob("php", "8.4.2")];
    backendWith({ jobs: () => jobs });
    renderPage();
    await within(await availableRow("8.4.2")).findByRole("progressbar");
    await liveEvents();

    jobs = [{ ...runtimeJob("php", "8.4.2"), state: "cancelled" }];
    emitEvent(JOB_DONE_EVENT, {
      jobId: "job-php-8.4.2",
      ok: false,
      error: { code: "CANCELLED", message: "the installation was cancelled" },
    });

    expect(await screen.findByText(/Installation of PHP 8.4.2 cancelled/)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      within(await availableRow("8.4.2")).getByRole("button", { name: "Install PHP 8.4.2" }),
    ).toBeEnabled();
  });

  it("explains a refused start, such as the same version already being removed", async () => {
    backendWith({
      more: {
        runtime_install: () => backendError("BUSY", "another operation is running for php 8.4.2"),
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Install PHP 8.4.2" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not install PHP 8.4.2");
    expect(alert).toHaveTextContent("PHP 8.4.2 is already being installed or removed");
    expect(alert).toHaveTextContent("another operation is running for php 8.4.2");
  });

  describe("a failed install", () => {
    const failed = (code: string, message: string, details?: Record<string, unknown>): JobInfo => ({
      ...runtimeJob("php", "8.4.2", { label: "Install PHP 8.4.2" }),
      state: "failed",
      error: { code, message, details } as JobInfo["error"],
    });

    it("says how much disk space is missing", async () => {
      backendWith({
        jobs: () => [
          failed("INSUFFICIENT_DISK", "need 524288000, have 104857600", {
            needed: 500 * MB,
            free: 100 * MB,
          }),
        ],
      });
      renderPage();

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Could not install PHP 8.4.2");
      expect(alert).toHaveTextContent("needs 500 MB and only 100 MB is free");
    });

    it("guides the user through a missing Visual C++ runtime, with a copyable address", async () => {
      backendWith({
        jobs: () => [
          failed("PREREQUISITE_MISSING", "Visual C++ runtime too old", {
            requirement: "vcredist-14-x64",
            name: "Microsoft Visual C++ Redistributable 2015-2022 (x64)",
            downloadUrl: "https://aka.ms/vs/17/release/vc_redist.x64.exe",
            minimum: "14.40",
            found: "14.38",
          }),
        ],
      });
      const user = userEvent.setup();
      renderPage();

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent(
        "PHP 8.4.2 needs Microsoft Visual C++ Redistributable 2015-2022 (x64) first.",
      );
      expect(alert).toHaveTextContent("found version 14.38 and needs 14.40 or newer");
      expect(alert).toHaveTextContent("https://aka.ms/vs/17/release/vc_redist.x64.exe");

      await user.click(within(alert).getByRole("button", { name: "Copy address" }));
      expect(await navigator.clipboard.readText()).toBe(
        "https://aka.ms/vs/17/release/vc_redist.x64.exe",
      );
    });

    it("says when the component is not installed at all", async () => {
      backendWith({
        jobs: () => [
          failed("PREREQUISITE_MISSING", "missing", {
            name: "Visual C++",
            downloadUrl: "https://example.com/vc",
            minimum: "14.40",
            found: null,
          }),
        ],
      });
      renderPage();
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "It is not installed on this computer.",
      );
    });

    it.each([
      ["CHECKSUM_MISMATCH", /did not match its checksum, so nothing was installed/],
      ["NETWORK", /Check your internet connection and try again/],
      ["NO_ARTIFACT_FOR_PLATFORM", /has no download for this computer/],
      ["ALREADY_INSTALLED", /PHP 8.4.2 is already installed/],
    ])("explains %s in plain words", async (code, text) => {
      backendWith({ jobs: () => [failed(code, "technical detail")] });
      renderPage();
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent(text);
      expect(alert).toHaveTextContent("technical detail");
    });

    it("falls back to the category and the technical message for a code it has no words for", async () => {
      backendWith({ jobs: () => [failed("ARCHIVE_UNSAFE", "entry escapes the folder")] });
      renderPage();
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Could not install PHP 8.4.2");
      expect(alert).toHaveTextContent("Unsafe download rejected");
      expect(alert).toHaveTextContent("entry escapes the folder");
    });

    it("can be dismissed", async () => {
      const jobs: JobInfo[] = [failed("NETWORK", "offline")];
      backendWith({ jobs: () => jobs });
      const user = userEvent.setup();
      renderPage();

      const alert = await screen.findByRole("alert");
      await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("does not show an attempt that a later success replaced", async () => {
      const newer: JobInfo = { ...runtimeJob("php", "8.4.2"), id: "newer", state: "succeeded" };
      const older = { ...failed("NETWORK", "offline"), id: "older" };
      backendWith({ jobs: () => [newer, older] });
      renderPage();

      await availableRow("8.4.2");
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  it("says when install progress cannot be shown because the task list failed", async () => {
    backendWith({ jobs: () => backendError("INTERNAL", "jobs unavailable") });
    renderPage();
    expect(await screen.findByText(/install progress cannot be shown/)).toBeInTheDocument();
  });
});

describe("uninstalling", () => {
  async function openDialog(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByRole("button", { name: "Uninstall PHP 8.3.9" }));
    return screen.findByRole("dialog", { name: "Uninstall PHP 8.3.9?" });
  }

  it("names what is affected and removes only after confirmation", async () => {
    backendWith({
      more: {
        runtime_usage: () => noUsage,
        runtime_uninstall: () => ({ id: "job-1", alreadyRunning: false }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    expect(dialog).toHaveTextContent(installedPhp.dir);
    expect(dialog).toHaveTextContent("Your project files are not touched");
    expect(callsTo("runtime_usage")).toEqual([[{ runtime: "php", version: "8.3.9" }]]);
    expect(callsTo("runtime_uninstall")).toHaveLength(0);

    await user.click(within(dialog).getByRole("button", { name: "Uninstall" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(callsTo("runtime_uninstall")).toEqual([
      [
        {
          runtime: "php",
          version: "8.3.9",
          deleteData: false,
          confirmBoundProjects: false,
        },
      ],
    ]);
  });

  it("keeps the confirm button off until it knows what uses the version", async () => {
    let answer: (report: UsageReport) => void = () => {};
    backendWith({
      more: {
        runtime_usage: () =>
          new Promise<UsageReport>((resolve) => {
            answer = resolve;
          }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    expect(
      within(dialog).getByRole("status", { name: "Checking what uses PHP 8.3.9" }),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeDisabled();

    answer(noUsage);
    await waitFor(() => {
      expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeEnabled();
    });
  });

  it("lists the projects that use the version and asks again in the button", async () => {
    backendWith({
      more: {
        runtime_usage: () => ({ ...noUsage, boundProjects: ["shop", "blog"] }),
        runtime_uninstall: () => ({ id: "job-1", alreadyRunning: false }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    expect(await within(dialog).findByText("These projects use this version")).toBeInTheDocument();
    expect(within(dialog).getByText("shop")).toBeInTheDocument();
    expect(within(dialog).getByText("blog")).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Uninstall anyway" }));

    await waitFor(() => {
      expect(callsTo("runtime_uninstall")).toHaveLength(1);
    });
    expect(callsTo("runtime_uninstall")[0]?.[0]).toMatchObject({ confirmBoundProjects: true });
  });

  it("refuses while a service runs from the version", async () => {
    backendWith({
      more: { runtime_usage: () => ({ ...noUsage, runningServices: ["php-fpm 8.3.9"] }) },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    expect(await within(dialog).findByText("Stop these services first")).toBeInTheDocument();
    expect(within(dialog).getByText("php-fpm 8.3.9")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeDisabled();
  });

  it("keeps databases by default and deletes them only when the box is ticked", async () => {
    backendWith({
      more: {
        runtime_usage: () => ({ ...noUsage, dataDirs: ["D:\\lüna home\\data\\php-8.3"] }),
        runtime_uninstall: () => ({ id: "job-1", alreadyRunning: false }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    const box = await within(dialog).findByRole("checkbox", {
      name: "Also delete the databases of this version",
    });
    expect(box).not.toBeChecked();
    expect(within(dialog).getByText("D:\\lüna home\\data\\php-8.3")).toBeInTheDocument();
    expect(within(dialog).getByText("They are kept unless you tick this box.")).toBeInTheDocument();

    await user.click(box);
    expect(
      within(dialog).getByText("Deleted databases cannot be brought back."),
    ).toBeInTheDocument();
    await user.click(
      within(dialog).getByRole("button", { name: "Uninstall and delete databases" }),
    );

    await waitFor(() => {
      expect(callsTo("runtime_uninstall")).toHaveLength(1);
    });
    expect(callsTo("runtime_uninstall")[0]?.[0]).toMatchObject({ deleteData: true });
  });

  it("cannot delete databases another installed version still uses", async () => {
    backendWith({
      more: {
        runtime_usage: () => ({ ...noUsage, dataDirs: ["D:\\data\\php-8.3"], dataShared: true }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    const box = await within(dialog).findByRole("checkbox");
    expect(box).toBeDisabled();
    expect(
      within(dialog).getByText(/Another installed version uses the same databases/),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeEnabled();
  });

  it("closes with Cancel or Escape without removing anything, and returns focus to the button", async () => {
    backendWith({ more: { runtime_usage: () => noUsage } });
    const user = userEvent.setup();
    renderPage();

    const opener = await screen.findByRole("button", { name: "Uninstall PHP 8.3.9" });
    const dialog = await openDialog(user);
    expect(within(dialog).getByRole("button", { name: "Keep it" })).toHaveFocus();

    await user.click(within(dialog).getByRole("button", { name: "Keep it" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(opener).toHaveFocus();

    await openDialog(user);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(callsTo("runtime_uninstall")).toHaveLength(0);
  });

  it("keeps Tab inside the dialog", async () => {
    backendWith({ more: { runtime_usage: () => noUsage } });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    const confirm = await within(dialog).findByRole("button", { name: "Uninstall" });
    await waitFor(() => {
      expect(confirm).toBeEnabled();
    });
    within(dialog).getByRole("button", { name: "Keep it" }).focus();
    await user.tab();
    expect(confirm).toHaveFocus();
    await user.tab();
    expect(within(dialog).getByRole("button", { name: "Keep it" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(confirm).toHaveFocus();
  });

  it("shows why the check failed and offers to retry", async () => {
    let attempts = 0;
    backendWith({
      more: {
        runtime_usage: () => {
          attempts += 1;
          return attempts === 1 ? backendError("IO", "cannot read the runtimes folder") : noUsage;
        },
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent("Could not check what uses PHP 8.3.9");
    expect(alert).toHaveTextContent("cannot read the runtimes folder");
    expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeDisabled();

    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    await waitFor(() => {
      expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeEnabled();
    });
  });

  it("keeps the dialog open and explains when the backend refuses", async () => {
    backendWith({
      more: {
        runtime_usage: () => noUsage,
        runtime_uninstall: () => backendError("BUSY", "another operation is running for php 8.3.9"),
      },
    });
    const user = userEvent.setup();
    renderPage();

    const dialog = await openDialog(user);
    await waitFor(() => {
      expect(within(dialog).getByRole("button", { name: "Uninstall" })).toBeEnabled();
    });
    await user.click(within(dialog).getByRole("button", { name: "Uninstall" }));

    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent("Could not uninstall PHP 8.3.9");
    expect(alert).toHaveTextContent("already being installed or removed");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("shows Removing... in the row while the uninstall runs, then a toast when it is done", async () => {
    let jobs: JobInfo[] = [
      runtimeJob("php", "8.3.9", { label: "Uninstall php 8.3.9", phase: "removing" }),
    ];
    let installed = [installedPhp];
    backendWith({ jobs: () => jobs, installed: () => installed });
    renderPage();

    const region = await screen.findByRole("region", { name: "Installed" });
    expect(await within(region).findByLabelText("Removing PHP 8.3.9")).toBeInTheDocument();
    expect(within(region).queryByRole("button", { name: /Uninstall/ })).not.toBeInTheDocument();
    await liveEvents();

    jobs = [{ ...jobs[0], state: "succeeded" } as JobInfo];
    installed = [];
    emitEvent(JOB_DONE_EVENT, { jobId: "job-php-8.3.9", ok: true });

    expect(await screen.findByText("PHP 8.3.9 uninstalled.")).toBeInTheDocument();
    expect(await screen.findByText("Nothing installed")).toBeInTheDocument();
  });

  it("shows a failed uninstall inline", async () => {
    backendWith({
      jobs: () => [
        {
          ...runtimeJob("php", "8.3.9", { label: "Uninstall php 8.3.9" }),
          state: "failed",
          error: {
            code: "RUNTIME_IN_USE",
            message: "a program is running from D:\\x",
            details: { path: "D:\\x" },
          },
        },
      ],
    });
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not uninstall PHP 8.3.9");
    expect(alert).toHaveTextContent("still running, so it cannot be removed");
  });
});
