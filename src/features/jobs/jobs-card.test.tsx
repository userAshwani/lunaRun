import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { JobInfo } from "@/bindings/JobInfo";
import { JOB_DONE_EVENT, JOB_PROGRESS_EVENT } from "@/lib/events";
import { createQueryClient } from "@/lib/query-client";
import {
  backendError,
  callsTo,
  emitEvent,
  listenerCount,
  resetBackend,
  respondTo,
} from "@/test/backend";
import { JobsCard } from "./jobs-card";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const MB = 1024 * 1024;

const job = (overrides: Partial<JobInfo> = {}): JobInfo => ({
  id: "job-1",
  key: "php@8.3.14",
  label: "Install PHP 8.3.14",
  state: "running",
  phase: "download",
  bytesDone: 40 * MB,
  bytesTotal: 100 * MB,
  message: null,
  error: null,
  ...overrides,
});

function renderCard() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <JobsCard />
    </QueryClientProvider>,
  );
}

/** Serve a mutable job list, so a test can change what the next `jobs_list` call returns. */
function serveJobs(initial: JobInfo[]) {
  const state = { jobs: initial, cancelled: [] as string[] };
  respondTo({
    jobs_list: () => state.jobs,
    job_cancel: (args) => {
      state.cancelled.push((args as { jobId: string }).jobId);
      return true;
    },
  });
  return state;
}

beforeEach(() => {
  resetBackend();
});

describe("JobsCard", () => {
  it("shows an honest empty state when there are no jobs", async () => {
    serveJobs([]);
    renderCard();
    expect(screen.getByRole("status", { name: "Loading background tasks" })).toBeInTheDocument();
    expect(await screen.findByText("No background tasks")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("shows a running job with phase, percentage, sizes and a cancel button", async () => {
    serveJobs([job()]);
    renderCard();

    expect(await screen.findByText("Install PHP 8.3.14")).toBeInTheDocument();
    expect(screen.getByText("Running")).toBeInTheDocument();
    const bar = screen.getByRole("progressbar", { name: "Progress of Install PHP 8.3.14" });
    expect(bar).toHaveAttribute("aria-valuenow", "40");
    expect(screen.getByText("Downloading")).toBeInTheDocument();
    expect(screen.getByText(/40%/)).toBeInTheDocument();
    expect(screen.getByText(/40\.0 MB of 100 MB/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel Install PHP 8.3.14" })).toBeInTheDocument();
  });

  it("shows an indeterminate bar when the total size is unknown", async () => {
    serveJobs([job({ phase: "extract", bytesTotal: null, bytesDone: 3 * MB })]);
    renderCard();
    const bar = await screen.findByRole("progressbar");
    expect(bar).not.toHaveAttribute("aria-valuenow");
    expect(screen.getByText("Extracting")).toBeInTheDocument();
    expect(screen.getByText(/3\.0 MB so far/)).toBeInTheDocument();
  });

  it("falls back to the raw phase name for a phase this build does not know", async () => {
    serveJobs([job({ phase: "compiling" })]);
    renderCard();
    expect(await screen.findByText("compiling")).toBeInTheDocument();
  });

  it("shows finished jobs without a progress bar or cancel button, and explains failures", async () => {
    serveJobs([
      job({ id: "job-3", label: "Install MySQL 8.4", state: "succeeded", bytesDone: 100 * MB }),
      job({
        id: "job-2",
        label: "Install Node 24",
        state: "failed",
        error: { code: "CHECKSUM_MISMATCH", message: "node.zip does not match its SHA-256" },
      }),
      job({
        id: "job-1",
        label: "Install PHP 8.1",
        state: "cancelled",
        error: { code: "CANCELLED", message: "cancelled" },
      }),
    ]);
    renderCard();

    expect(await screen.findByText("Done")).toBeInTheDocument();
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Cancelled")).toBeInTheDocument();
    expect(screen.getByText(/Download failed verification/)).toBeInTheDocument();
    expect(screen.getByText(/node\.zip does not match its SHA-256/)).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Cancel/ })).not.toBeInTheDocument();
  });

  it("updates progress live from events, without asking the backend again", async () => {
    serveJobs([job()]);
    renderCard();
    await screen.findByText("Install PHP 8.3.14");
    await waitFor(() => {
      expect(listenerCount(JOB_PROGRESS_EVENT)).toBe(1);
    });
    expect(callsTo("jobs_list")).toHaveLength(1);

    act(() => {
      emitEvent(JOB_PROGRESS_EVENT, {
        jobId: "job-1",
        phase: "extract",
        bytesDone: 75 * MB,
        bytesTotal: 100 * MB,
        message: null,
      });
    });

    // React Query tells its subscribers on the next tick, so wait for the render.
    await waitFor(() => {
      expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "75");
    });
    expect(screen.getByText("Extracting")).toBeInTheDocument();
    expect(callsTo("jobs_list")).toHaveLength(1);
  });

  it("refetches when an event names a job it has not listed", async () => {
    const backend = serveJobs([]);
    renderCard();
    await screen.findByText("No background tasks");
    await waitFor(() => {
      expect(listenerCount(JOB_PROGRESS_EVENT)).toBe(1);
    });

    backend.jobs = [job({ id: "job-7", label: "Install Caddy" })];
    act(() => {
      emitEvent(JOB_PROGRESS_EVENT, {
        jobId: "job-7",
        phase: "starting",
        bytesDone: 0,
        bytesTotal: null,
        message: null,
      });
    });

    expect(await screen.findByText("Install Caddy")).toBeInTheDocument();
    expect(callsTo("jobs_list")).toHaveLength(2);
  });

  it("refetches when a job finishes and shows its final state", async () => {
    const backend = serveJobs([job()]);
    renderCard();
    await screen.findByText("Running");
    await waitFor(() => {
      expect(listenerCount(JOB_DONE_EVENT)).toBe(1);
    });

    backend.jobs = [job({ state: "succeeded", bytesDone: 100 * MB })];
    act(() => {
      emitEvent(JOB_DONE_EVENT, { jobId: "job-1", ok: true, error: null });
    });

    expect(await screen.findByText("Done")).toBeInTheDocument();
    expect(screen.queryByText("Running")).not.toBeInTheDocument();
  });

  it("cancels the right job and then refreshes the list", async () => {
    const backend = serveJobs([
      job({ id: "job-2", label: "Install Node 24" }),
      job({ id: "job-1", label: "Install PHP 8.3.14" }),
    ]);
    renderCard();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Cancel Install PHP 8.3.14" }));

    await waitFor(() => {
      expect(backend.cancelled).toEqual(["job-1"]);
    });
    expect(callsTo("job_cancel")).toEqual([[{ jobId: "job-1" }]]);
    await waitFor(() => {
      expect(callsTo("jobs_list").length).toBeGreaterThanOrEqual(2);
    });
  });

  it("shows what failed and retries on request when the list cannot be loaded", async () => {
    let attempts = 0;
    respondTo({
      jobs_list: () => {
        attempts += 1;
        return attempts === 1 ? backendError("IO", "the job store is unavailable") : [];
      },
    });
    renderCard();
    const user = userEvent.setup();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not load background tasks");
    expect(alert).toHaveTextContent("File system error");
    expect(alert).toHaveTextContent("the job store is unavailable");

    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("No background tasks")).toBeInTheDocument();
  });

  it("stops listening when it is removed from the screen", async () => {
    serveJobs([]);
    const view = renderCard();
    await waitFor(() => {
      expect(listenerCount(JOB_PROGRESS_EVENT)).toBe(1);
      expect(listenerCount(JOB_DONE_EVENT)).toBe(1);
    });

    view.unmount();
    expect(listenerCount(JOB_PROGRESS_EVENT)).toBe(0);
    expect(listenerCount(JOB_DONE_EVENT)).toBe(0);
  });

  it("does not leak a subscription if it is removed before the subscription is ready", async () => {
    serveJobs([]);
    renderCard().unmount(); // unmounted while `listen` is still resolving
    await act(async () => {
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(listenerCount(JOB_PROGRESS_EVENT)).toBe(0);
      expect(listenerCount(JOB_DONE_EVENT)).toBe(0);
    });
  });
});
