import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LogLine } from "@/bindings/LogLine";
import type { ServiceLogs } from "@/bindings/ServiceLogs";
import { SERVICE_LOG_EVENT, SERVICE_STATE_EVENT } from "@/lib/events";
import { backendError, callsTo, emitEvent, listenerCount, resetBackend } from "@/test/backend";
import {
  backendWith,
  caddyRow,
  mysqlRow,
  phpPool,
  renderPage,
  status,
} from "@/test/services-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

beforeEach(() => {
  resetBackend();
});

const T0 = Date.UTC(2026, 10, 12, 14, 3, 7);

function line(text: string, source = "output", ts = T0): LogLine {
  return { source, line: text, ts };
}

function logsOf(service: string, sources: Record<string, LogLine[]>): ServiceLogs {
  return {
    service,
    sources: Object.entries(sources).map(([source, lines]) => ({ source, lines })),
  };
}

const mysql = logsOf("mysql@8.4", {
  output: [line("[lunaRun] starting"), line("mysqld ready for connections", "output", T0 + 1000)],
  "MySQL error log": [line("InnoDB: initialization complete", "MySQL error log", T0 + 500)],
});

function backend(snapshot: () => unknown, rows: unknown[] = [mysqlRow]) {
  backendWith({
    list: () => rows,
    more: { service_logs: snapshot },
  });
}

async function openLogs(name: string) {
  const user = userEvent.setup();
  renderPage();
  await user.click(await screen.findByRole("button", { name: `Show the logs of ${name}` }));
  return user;
}

/** Waits until the page listens to both of the events the viewer lives on. */
async function live() {
  await waitFor(() => {
    expect(listenerCount(SERVICE_LOG_EVENT)).toBe(1);
  });
}

describe("the log viewer", () => {
  it("opens under the table, asks for the last 2000 lines, and shows a tab per source", async () => {
    backend(() => [mysql]);
    await openLogs("MySQL 8.4");

    expect(await screen.findByRole("heading", { name: "Logs of MySQL 8.4" })).toBeInTheDocument();
    expect(callsTo("service_logs")).toEqual([[{ id: "mysql@8.4", tailLines: 2000 }]]);
    const tabs = screen.getByRole("tablist", { name: "Log sources of MySQL 8.4" });
    expect(
      within(tabs)
        .getAllByRole("tab")
        .map((tab) => tab.textContent),
    ).toEqual(["Output", "MySQL error log"]);
    const log = screen.getByRole("log", { name: "Log lines" });
    expect(within(log).getByText("[lunaRun] starting")).toBeInTheDocument();
    expect(within(log).getByText("mysqld ready for connections")).toBeInTheDocument();
    expect(screen.getByText("2 lines")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide the logs of MySQL 8.4" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("shows the lines of the tab that is chosen", async () => {
    backend(() => [mysql]);
    const user = await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });

    await user.click(screen.getByRole("tab", { name: "MySQL error log" }));

    const log = screen.getByRole("log", { name: "Log lines" });
    expect(within(log).getByText("InnoDB: initialization complete")).toBeInTheDocument();
    expect(within(log).queryByText("[lunaRun] starting")).not.toBeInTheDocument();
    expect(screen.getByText("1 line")).toBeInTheDocument();
  });

  it("gives each worker of a Windows pool its own tabs", async () => {
    const workers = [1, 2].map((n) =>
      logsOf(`php-fcgi@8.3.36-w${String(n)}`, {
        output: [line(`worker ${String(n)} up`)],
      }),
    );
    backend(() => workers, [phpPool]);
    const user = await openLogs("PHP 8.3.36");

    const tabs = await screen.findAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["Worker 1: Output", "Worker 2: Output"]);
    expect(screen.getByText("worker 1 up")).toBeInTheDocument();
    await user.click(tabs[1] as HTMLElement);
    expect(screen.getByText("worker 2 up")).toBeInTheDocument();
  });

  it("appends new lines as they arrive, from the service of this row only", async () => {
    backend(() => [mysql]);
    await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();

    emitEvent(SERVICE_LOG_EVENT, {
      id: "mysql@8.4",
      lines: [line("a new query", "output", T0 + 5000), line("another", "output", T0 + 5001)],
    });
    emitEvent(SERVICE_LOG_EVENT, { id: "caddy", lines: [line("not for this panel")] });

    const log = screen.getByRole("log", { name: "Log lines" });
    expect(await within(log).findByText("another")).toBeInTheDocument();
    expect(within(log).getByText("a new query")).toBeInTheDocument();
    expect(within(log).queryByText("not for this panel")).not.toBeInTheDocument();
    expect(screen.getByText("4 lines")).toBeInTheDocument();
  });

  it("does not show a line twice when it is in the snapshot and arrives as an event too", async () => {
    backend(() => [mysql]);
    await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();

    emitEvent(SERVICE_LOG_EVENT, {
      id: "mysql@8.4",
      lines: [
        line("mysqld ready for connections", "output", T0 + 1000),
        line("fresh", "output", T0 + 2000),
      ],
    });

    await screen.findByText("fresh");
    expect(screen.getAllByText("mysqld ready for connections")).toHaveLength(1);
    expect(screen.getByText("3 lines")).toBeInTheDocument();
  });

  it("makes a tab for a log file that shows up later", async () => {
    backend(() => [logsOf("mysql@8.4", { output: [line("hello")] })]);
    await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();
    expect(screen.getAllByRole("tab")).toHaveLength(1);

    emitEvent(SERVICE_LOG_EVENT, {
      id: "mysql@8.4",
      lines: [line("slow query", "MySQL slow log", T0 + 9000)],
    });

    expect(await screen.findByRole("tab", { name: "MySQL slow log" })).toBeInTheDocument();
  });

  it("starts over when the service is started again, because its log starts over", async () => {
    let snapshot: ServiceLogs[] = [mysql];
    backend(() => snapshot);
    await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();
    await waitFor(() => {
      expect(listenerCount(SERVICE_STATE_EVENT)).toBeGreaterThan(0);
    });
    emitEvent(SERVICE_LOG_EVENT, {
      id: "mysql@8.4",
      lines: [line("old run", "output", T0 + 3000)],
    });
    await screen.findByText("old run");

    snapshot = [
      logsOf("mysql@8.4", { output: [line("[lunaRun] starting", "output", T0 + 60_000)] }),
    ];
    emitEvent(SERVICE_STATE_EVENT, status("mysql@8.4", { state: "starting" }));

    await waitFor(() => {
      expect(screen.queryByText("old run")).not.toBeInTheDocument();
    });
    expect(callsTo("service_logs")).toHaveLength(2);
    expect(await screen.findByText("1 line")).toBeInTheDocument();
  });

  it("does not start over because another service started", async () => {
    backend(() => [mysql]);
    await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();
    await waitFor(() => {
      expect(listenerCount(SERVICE_STATE_EVENT)).toBeGreaterThan(0);
    });
    emitEvent(SERVICE_LOG_EVENT, { id: "mysql@8.4", lines: [line("kept", "output", T0 + 3000)] });
    await screen.findByText("kept");

    emitEvent(SERVICE_STATE_EVENT, status("caddy", { state: "starting" }));
    emitEvent(SERVICE_STATE_EVENT, status("mysql@8.4-extra", { state: "starting" }));

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.getByText("kept")).toBeInTheDocument();
    expect(callsTo("service_logs")).toHaveLength(1);
  });

  it("keeps no more than 2000 lines of a source", async () => {
    backend(() => [logsOf("mysql@8.4", { output: [line("first")] })]);
    await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();

    emitEvent(SERVICE_LOG_EVENT, {
      id: "mysql@8.4",
      lines: Array.from({ length: 2500 }, (_, n) => line(`line ${String(n)}`, "output", T0 + n)),
    });

    expect(await screen.findByText("2000 lines")).toBeInTheDocument();
    expect(screen.queryByText("first")).not.toBeInTheDocument();
    expect(screen.getByText("line 2499")).toBeInTheDocument();
  });

  it("says plainly that a service that was never started has printed nothing", async () => {
    backend(() => [logsOf("caddy", {})], [caddyRow]);
    await openLogs("Caddy");

    expect(await screen.findByText(/has not been started yet/)).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("says that a source is empty so far", async () => {
    backend(() => [logsOf("caddy", { output: [] })], [caddyRow]);
    await openLogs("Caddy");

    expect(await screen.findByText(/Nothing printed yet/)).toBeInTheDocument();
    expect(screen.getByText("0 lines")).toBeInTheDocument();
  });

  it("shows a skeleton while loading and a retry when the log cannot be read", async () => {
    let attempt = 0;
    backend(() => {
      attempt += 1;
      return attempt === 1 ? backendError("IO", "logs/mysql@8.4.log: access denied") : [mysql];
    });
    const user = await openLogs("MySQL 8.4");

    expect(await screen.findByText("Could not load the log")).toBeInTheDocument();
    expect(screen.getByText(/access denied/)).toBeInTheDocument();
    await user.click(within(screen.getByRole("alert")).getByRole("button", { name: "Try again" }));

    expect(await screen.findByText("mysqld ready for connections")).toBeInTheDocument();
    expect(screen.queryByText("Could not load the log")).not.toBeInTheDocument();
  });

  it("closes with its button and with the row's own toggle", async () => {
    backend(() => [mysql]);
    const user = await openLogs("MySQL 8.4");
    await screen.findByRole("heading", { name: "Logs of MySQL 8.4" });

    await user.click(screen.getByRole("button", { name: "Close the logs of MySQL 8.4" }));
    expect(screen.queryByRole("heading", { name: "Logs of MySQL 8.4" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show the logs of MySQL 8.4" }));
    await screen.findByRole("heading", { name: "Logs of MySQL 8.4" });
    await user.click(screen.getByRole("button", { name: "Hide the logs of MySQL 8.4" }));
    expect(screen.queryByRole("heading", { name: "Logs of MySQL 8.4" })).not.toBeInTheDocument();
  });

  it("shows the log of one row at a time", async () => {
    backendWith({
      list: () => [caddyRow, mysqlRow],
      more: {
        service_logs: (args) =>
          (args as { id: string }).id === "caddy"
            ? [logsOf("caddy", { output: [line("caddy says hi")] })]
            : [mysql],
      },
    });
    const user = userEvent.setup();
    renderPage();
    await user.click(await screen.findByRole("button", { name: "Show the logs of Caddy" }));
    await screen.findByText("caddy says hi");

    await user.click(screen.getByRole("button", { name: "Show the logs of MySQL 8.4" }));

    expect(await screen.findByText("mysqld ready for connections")).toBeInTheDocument();
    expect(screen.queryByText("caddy says hi")).not.toBeInTheDocument();
    expect(screen.getAllByRole("heading", { name: /^Logs of/ })).toHaveLength(1);
  });

  it("stops following when the user scrolls up, and jumps back on request", async () => {
    backend(() => [mysql]);
    const user = await openLogs("MySQL 8.4");
    const log = await screen.findByRole("log", { name: "Log lines" });
    Object.defineProperty(log, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(log, "clientHeight", { configurable: true, value: 100 });
    expect(screen.queryByRole("button", { name: "Jump to latest" })).not.toBeInTheDocument();

    log.scrollTop = 0;
    fireEvent.scroll(log);

    const jump = await screen.findByRole("button", { name: "Jump to latest" });
    await user.click(jump);
    expect(screen.queryByRole("button", { name: "Jump to latest" })).not.toBeInTheDocument();
    expect(log.scrollTop).toBe(1000);
  });

  it("stops listening when the panel closes", async () => {
    backend(() => [mysql]);
    const user = await openLogs("MySQL 8.4");
    await screen.findByRole("log", { name: "Log lines" });
    await live();

    await user.click(screen.getByRole("button", { name: "Close the logs of MySQL 8.4" }));

    await waitFor(() => {
      expect(listenerCount(SERVICE_LOG_EVENT)).toBe(0);
    });
  });
});
