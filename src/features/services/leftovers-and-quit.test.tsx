import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ShutdownProgress } from "@/bindings/ShutdownProgress";
import { createQueryClient } from "@/lib/query-client";
import { SERVICE_LOG_EVENT, SHUTDOWN_PROGRESS_EVENT } from "@/lib/events";
import { backendError, callsTo, emitEvent, listenerCount, resetBackend } from "@/test/backend";
import { backendWith, leftover, renderPage } from "@/test/services-fixtures";
import { QuitOverlay } from "./quit-overlay";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

beforeEach(() => {
  resetBackend();
});

describe("services left from an earlier session", () => {
  it("adopts one: the command gets the service's id, a toast confirms, and the lists load again", async () => {
    let left = [leftover];
    backendWith({
      leftovers: () => left,
      more: {
        service_adopt: () => {
          left = [];
          return null;
        },
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Adopt MySQL 8.4" }));

    expect(callsTo("service_adopt")).toEqual([[{ id: "mysql@8.4" }]]);
    expect(await screen.findByText("MySQL 8.4 adopted")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByText(/earlier session/)).not.toBeInTheDocument();
    });
  });

  it("asks before stopping one, names it, and does nothing when the user keeps it", async () => {
    backendWith({ leftovers: () => [leftover], more: { service_stop_leftover: () => null } });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Stop MySQL 8.4" }));

    const dialog = await screen.findByRole("dialog", { name: "Stop MySQL 8.4?" });
    expect(within(dialog).getByText(/ends it if it has not left after ten seconds/)).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Keep it running" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(callsTo("service_stop_leftover")).toEqual([]);
  });

  it("stops one after the user confirms", async () => {
    let left = [leftover];
    backendWith({
      leftovers: () => left,
      more: {
        service_stop_leftover: () => {
          left = [];
          return null;
        },
      },
    });
    const user = userEvent.setup();
    renderPage();
    await user.click(await screen.findByRole("button", { name: "Stop MySQL 8.4" }));

    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Stop it" }),
    );

    expect(callsTo("service_stop_leftover")).toEqual([[{ id: "mysql@8.4" }]]);
    expect(await screen.findByText("MySQL 8.4 stopped")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByText(/earlier session/)).not.toBeInTheDocument();
    });
  });

  it("keeps the service listed with the reason when it is refused, and lets the user try again", async () => {
    backendWith({
      leftovers: () => [leftover],
      more: {
        service_adopt: () =>
          backendError("INVALID_PATH", "mysql@8.4 was started from /a/mysqld, not from /b/mysqld"),
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Adopt MySQL 8.4" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("MySQL 8.4: Invalid path.");
    expect(alert).toHaveTextContent("was started from /a/mysqld, not from /b/mysqld");
    expect(screen.getByText("MySQL 8.4 (process 4242)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Adopt MySQL 8.4" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Stop MySQL 8.4" })).toBeEnabled();
  });

  it("keeps both buttons off while one of them is under way", async () => {
    let finish: (value: unknown) => void = () => {};
    backendWith({
      leftovers: () => [leftover],
      more: {
        service_adopt: () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Adopt MySQL 8.4" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Adopt MySQL 8.4" })).toBeDisabled();
    });
    expect(screen.getByRole("button", { name: "Stop MySQL 8.4" })).toBeDisabled();
    finish(null);
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Stop MySQL 8.4" })).toBeEnabled();
    });
  });

  it("names a service that is not in the list by its id", async () => {
    backendWith({
      list: () => [],
      leftovers: () => [{ ...leftover, id: "mariadb@12.3" }],
    });
    renderPage();

    expect(await screen.findByText("mariadb@12.3 (process 4242)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Adopt mariadb@12.3" })).toBeInTheDocument();
  });
});

function progress(extra: Partial<ShutdownProgress>): ShutdownProgress {
  return {
    group: "web",
    total: 1,
    done: 0,
    remaining: ["caddy"],
    finished: null,
    elapsedMs: 0,
    ...extra,
  };
}

function renderOverlay() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <QuitOverlay />
    </QueryClientProvider>,
  );
}

async function listening() {
  await waitFor(() => {
    expect(listenerCount(SHUTDOWN_PROGRESS_EVENT)).toBe(1);
  });
}

describe("the quit overlay", () => {
  it("names the shutdown event the backend sends", () => {
    expect(SHUTDOWN_PROGRESS_EVENT).toBe("shutdown:progress");
    expect(SERVICE_LOG_EVENT).toBe("service:log");
  });

  it("shows nothing until the first progress arrives, so a quit with nothing to stop never flashes it", async () => {
    resetBackend();
    renderOverlay();
    await listening();

    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("says that services are being stopped, which group, how far, and what it waits for", async () => {
    renderOverlay();
    await listening();

    emitEvent(
      SHUTDOWN_PROGRESS_EVENT,
      progress({ group: "databases", total: 2, done: 1, remaining: ["mysql@8.4"] }),
    );

    const dialog = await screen.findByRole("alertdialog", { name: "Stopping your services" });
    expect(within(dialog).getByText(/lunaRun closes when they have stopped/)).toBeInTheDocument();
    expect(within(dialog).getByText("Databases")).toBeInTheDocument();
    expect(within(dialog).getByText("1 of 2 stopped")).toBeInTheDocument();
    expect(within(dialog).getByText("Waiting for mysql@8.4")).toBeInTheDocument();
    expect(within(dialog).getByRole("progressbar", { name: "Stopping Databases" })).toHaveAttribute(
      "aria-valuenow",
      "50",
    );
  });

  it("lists the groups in the order they are stopped, and marks a finished one", async () => {
    renderOverlay();
    await listening();

    emitEvent(SHUTDOWN_PROGRESS_EVENT, progress({ group: "databases", remaining: ["mysql@8.4"] }));
    emitEvent(
      SHUTDOWN_PROGRESS_EVENT,
      progress({ group: "web", done: 1, remaining: [], finished: "caddy" }),
    );

    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() => {
      expect(within(dialog).getAllByRole("listitem")).toHaveLength(2);
    });
    const items = within(dialog).getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Web server");
    expect(items[0]).toHaveTextContent("1 of 1 stopped");
    expect(within(items[0] as HTMLElement).getByText("done")).toBeInTheDocument();
    expect(within(items[1] as HTMLElement).queryByText("done")).not.toBeInTheDocument();
    expect(items[0]).not.toHaveTextContent("Waiting for");
    expect(items[1]).toHaveTextContent("Databases");
    expect(items[1]).toHaveTextContent("Waiting for mysql@8.4");
  });

  it("stops listening when it goes away", async () => {
    const view = renderOverlay();
    await listening();

    view.unmount();

    await waitFor(() => {
      expect(listenerCount(SHUTDOWN_PROGRESS_EVENT)).toBe(0);
    });
  });
});
