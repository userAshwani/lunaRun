import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { JobInfo } from "@/bindings/JobInfo";
import type { ServiceRow } from "@/bindings/ServiceRow";
import { JOB_DONE_EVENT } from "@/lib/events";
import { backendError, callsTo, emitEvent, listenerCount, resetBackend } from "@/test/backend";
import {
  backendWith,
  caddyRow,
  mysqlRow,
  phpPool,
  renderPage,
  serviceJob,
  status,
} from "@/test/services-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

beforeEach(() => {
  resetBackend();
});

const runningCaddy: ServiceRow = {
  ...caddyRow,
  state: "running",
  running: 1,
  ports: [2119],
  members: [status("caddy", { state: "running", pid: 99 })],
};

/** The page listens for the end of a job twice: the task list and the effects (reload, toast). */
async function liveEvents() {
  await waitFor(() => {
    expect(listenerCount(JOB_DONE_EVENT)).toBe(2);
  });
}

describe("the buttons of a service", () => {
  it("offers Start on a stopped service and nothing else", async () => {
    backendWith({ list: () => [caddyRow] });
    renderPage();

    expect(await screen.findByRole("button", { name: "Start Caddy" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /^(Stop|Restart) Caddy/ })).not.toBeInTheDocument();
  });

  it("offers Restart and Stop on a running service, and no Start", async () => {
    backendWith({ list: () => [runningCaddy] });
    renderPage();

    expect(await screen.findByRole("button", { name: "Restart Caddy" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Stop Caddy" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Start Caddy" })).not.toBeInTheDocument();
  });

  it("offers only Stop on a service that is starting by itself", async () => {
    backendWith({ list: () => [phpPool] });
    renderPage();

    expect(await screen.findByRole("button", { name: "Stop PHP 8.3.36" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /^(Start|Restart) PHP/ })).not.toBeInTheDocument();
  });

  it("shows a disabled Stopping while a service stops", async () => {
    backendWith({ list: () => [{ ...runningCaddy, state: "stopping" }] });
    renderPage();

    expect(await screen.findByRole("button", { name: "Caddy is stopping" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /^(Start|Stop|Restart) Caddy/ })).toBeNull();
  });

  it("offers Start again after a failure or a crash", async () => {
    backendWith({
      list: () => [
        { ...caddyRow, state: "error" },
        { ...mysqlRow, state: "crashed", needsSetup: false },
      ],
    });
    renderPage();

    expect(await screen.findByRole("button", { name: "Start Caddy" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Start MySQL 8.4" })).toBeEnabled();
  });
});

describe("starting, stopping and restarting", () => {
  it("sends the row's id to the command it is for", async () => {
    backendWith({
      list: () => [runningCaddy, mysqlRow],
      more: {
        service_start: () => ({ id: "job-1", alreadyRunning: false }),
        service_stop: () => ({ id: "job-2", alreadyRunning: false }),
        service_restart: () => ({ id: "job-3", alreadyRunning: false }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Start MySQL 8.4" }));
    await user.click(screen.getByRole("button", { name: "Stop Caddy" }));
    await user.click(screen.getByRole("button", { name: "Restart Caddy" }));

    expect(callsTo("service_start")).toEqual([[{ id: "mysql@8.4" }]]);
    expect(callsTo("service_stop")).toEqual([[{ id: "caddy" }]]);
    expect(callsTo("service_restart")).toEqual([[{ id: "caddy" }]]);
  });

  it("shows Cancel while its start runs, and cancelling asks the backend to cancel that job", async () => {
    let jobs: JobInfo[] = [];
    backendWith({
      list: () => [caddyRow],
      jobs: () => jobs,
      more: {
        service_start: () => {
          jobs = [serviceJob("caddy", "Start")];
          return { id: "job-start-caddy", alreadyRunning: false };
        },
        job_cancel: () => true,
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Start Caddy" }));
    const cancel = await screen.findByRole("button", { name: "Cancel starting Caddy" });
    expect(screen.queryByRole("button", { name: "Start Caddy" })).not.toBeInTheDocument();
    await user.click(cancel);

    expect(callsTo("job_cancel")).toEqual([[{ jobId: "job-start-caddy" }]]);
  });

  it("explains the first start of a database while it sets up, with a progress bar", async () => {
    backendWith({
      list: () => [{ ...mysqlRow, state: "starting" }],
      jobs: () => [serviceJob("mysql@8.4", "Start", { phase: "setup" })],
    });
    renderPage();

    expect(await screen.findByText(/Setting up its data folder\. This only happens/)).toBeVisible();
    expect(screen.getByRole("progressbar", { name: "Setting up MySQL 8.4" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel starting MySQL 8.4" })).toBeEnabled();
  });

  it("says nothing about setting up while the service is only waiting to answer", async () => {
    backendWith({
      list: () => [{ ...mysqlRow, state: "starting" }],
      jobs: () => [serviceJob("mysql@8.4", "Start", { phase: "starting" })],
    });
    renderPage();

    expect(await screen.findByRole("button", { name: "Cancel starting MySQL 8.4" })).toBeEnabled();
    expect(screen.queryByText(/Setting up its data folder\. This only happens/)).toBeNull();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("does not show a cancelled or a finished job as a problem", async () => {
    backendWith({
      list: () => [caddyRow, mysqlRow],
      jobs: () => [
        serviceJob("caddy", "Start", {
          state: "cancelled",
          error: { code: "CANCELLED", message: "starting caddy was cancelled" },
        }),
        serviceJob("mysql@8.4", "Start", { state: "succeeded" }),
      ],
    });
    renderPage();

    expect(await screen.findByRole("button", { name: "Start Caddy" })).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("does not offer Cancel for a stop, which must not be interrupted", async () => {
    backendWith({
      list: () => [runningCaddy],
      jobs: () => [serviceJob("caddy", "Stop", { phase: "stopping" })],
    });
    renderPage();

    expect(await screen.findByRole("button", { name: "Caddy is stopping" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /Cancel/ })).not.toBeInTheDocument();
  });

  it("confirms with a toast and loads the list again when a start is done", async () => {
    let running = false;
    backendWith({
      list: () => [running ? runningCaddy : caddyRow],
      jobs: () => [serviceJob("caddy", "Start")],
    });
    renderPage();
    await screen.findByRole("button", { name: "Cancel starting Caddy" });
    await liveEvents();
    const loads = callsTo("services_list").length;

    running = true;
    emitEvent(JOB_DONE_EVENT, { jobId: "job-start-caddy", ok: true, error: null });

    expect(await screen.findByText("Caddy started")).toBeInTheDocument();
    await waitFor(() => {
      expect(callsTo("services_list").length).toBeGreaterThan(loads);
    });
  });

  it.each([
    ["Stop", "Caddy stopped"],
    ["Restart", "Caddy restarted"],
  ] as const)("confirms a finished %s with its own words", async (action, text) => {
    backendWith({ list: () => [runningCaddy], jobs: () => [serviceJob("caddy", action)] });
    renderPage();
    await screen.findByRole("table");
    await liveEvents();

    emitEvent(JOB_DONE_EVENT, {
      jobId: `job-${action.toLowerCase()}-caddy`,
      ok: true,
      error: null,
    });

    expect(await screen.findByText(text)).toBeInTheDocument();
  });

  it("keeps the buttons of a service waiting while its command is on its way", async () => {
    let answer: (value: unknown) => void = () => {};
    backendWith({
      list: () => [runningCaddy],
      more: {
        service_stop: () =>
          new Promise((resolve) => {
            answer = resolve;
          }),
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Stop Caddy" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Stop Caddy" })).toBeDisabled();
    });
    expect(screen.getByRole("button", { name: "Restart Caddy" })).toBeDisabled();
    answer({ id: "job-1", alreadyRunning: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Stop Caddy" })).toBeEnabled();
    });
  });

  it("says a cancelled start was cancelled, as a note and not as a problem", async () => {
    backendWith({ list: () => [caddyRow], jobs: () => [serviceJob("caddy", "Start")] });
    renderPage();
    await screen.findByRole("button", { name: "Cancel starting Caddy" });
    await liveEvents();

    emitEvent(JOB_DONE_EVENT, {
      jobId: "job-start-caddy",
      ok: false,
      error: { code: "CANCELLED", message: "starting caddy was cancelled" },
    });

    expect(await screen.findByText("Starting Caddy was cancelled")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("ignores jobs that are not about a service", async () => {
    backendWith({
      list: () => [caddyRow],
      jobs: () => [
        {
          ...serviceJob("caddy", "Start"),
          id: "job-install",
          key: "php@8.4.2",
          label: "Install PHP 8.4.2",
        },
      ],
    });
    renderPage();

    expect(await screen.findByRole("button", { name: "Start Caddy" })).toBeEnabled();
  });
});

describe("what went wrong", () => {
  function failed(
    id: string,
    error: JobInfo["error"],
    action: "Start" | "Stop" | "Restart" = "Start",
  ) {
    return serviceJob(id, action, { state: "failed", error });
  }

  it("names who holds a taken port, and offers to use another one", async () => {
    backendWith({
      list: () => [{ ...mysqlRow, needsSetup: false }],
      jobs: () => [
        failed("mysql@8.4", {
          code: "PORT_IN_USE",
          message: "port 3306 is already in use by mysqld.exe (C:\\xampp\\mysql\\bin\\mysqld.exe)",
          details: {
            port: 3306,
            pid: 4312,
            processName: "mysqld.exe",
            processPath: "C:\\xampp\\mysql\\bin\\mysqld.exe",
          },
        }),
      ],
      more: { service_port_reassign: () => ({ id: "job-9", alreadyRunning: false }) },
    });
    const user = userEvent.setup();
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("MySQL 8.4 did not start")).toBeInTheDocument();
    expect(
      within(alert).getByText(
        "Port 3306 is used by mysqld.exe (C:\\xampp\\mysql\\bin\\mysqld.exe).",
      ),
    ).toBeInTheDocument();
    expect(within(alert).getByText(/That looks like XAMPP/)).toBeInTheDocument();
    await user.click(within(alert).getByRole("button", { name: "Use another port" }));

    expect(callsTo("service_port_reassign")).toEqual([[{ id: "mysql@8.4" }]]);
  });

  it("says why a port is taken when no program can be named", async () => {
    backendWith({
      list: () => [mysqlRow],
      jobs: () => [
        failed("mysql@8.4", {
          code: "PORT_IN_USE",
          message: "port 3306 is already in use (Address already in use)",
          details: { port: 3306, reason: "Address already in use" },
        }),
      ],
    });
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(
      within(alert).getByText("Port 3306 is already in use (Address already in use)."),
    ).toBeInTheDocument();
    expect(
      within(alert).getByText(/Stop that program, or let lunaRun use another port/),
    ).toBeVisible();
  });

  it("gives Caddy no other port to move to, and says what to do instead", async () => {
    backendWith({
      list: () => [caddyRow],
      jobs: () => [
        failed("caddy", {
          code: "PORT_IN_USE",
          message: "port 2119 is already in use by other.exe",
          details: { port: 2119, pid: 7, processName: "other.exe" },
        }),
      ],
    });
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Port 2119 is used by other.exe.")).toBeInTheDocument();
    expect(within(alert).getByText(/admin port cannot move/)).toBeInTheDocument();
    expect(within(alert).queryByRole("button", { name: "Use another port" })).toBeNull();
  });

  it("shows the end of what a failed database setup printed", async () => {
    backendWith({
      list: () => [mysqlRow],
      jobs: () => [
        failed("mysql@8.4", {
          code: "DATABASE_INIT_FAILED",
          message: "MySQL 8.4: the setup program failed with exit code 3",
          details: {
            service: "mysql@8.4",
            log: "C:\\lüna home\\logs\\mysql-8.4.init.log",
            tail: ["[ERROR] Can't create directory", "boom: cannot create the system tables"],
          },
        }),
      ],
    });
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/Setting up its data folder failed/)).toBeInTheDocument();
    const output = within(alert).getByLabelText("Output");
    expect(output).toHaveTextContent("boom: cannot create the system tables");
    expect(output).toHaveTextContent("[ERROR] Can't create directory");
    expect(within(alert).getByText(/mysql-8\.4\.init\.log/)).toBeInTheDocument();
  });

  it("shows what Caddy said when it rejects its settings", async () => {
    backendWith({
      list: () => [caddyRow],
      jobs: () => [
        failed("caddy", {
          code: "CONFIG_INVALID",
          message: "Caddy rejected the Caddyfile",
          details: { tail: ["adapting config: unexpected token"] },
        }),
      ],
    });
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(
      within(alert).getByText(/Caddy rejected the configuration lunaRun generated/),
    ).toBeVisible();
    expect(within(alert).getByLabelText("Output")).toHaveTextContent("unexpected token");
  });

  it("sends to Runtimes when the version a service runs from is gone or damaged", async () => {
    backendWith({
      list: () => [caddyRow, mysqlRow],
      jobs: () => [
        failed("caddy", {
          code: "RUNTIME_BROKEN",
          message: "the php.ini is missing",
        }),
        failed("mysql@8.4", {
          code: "NOT_FOUND",
          message: "mysql@8.4 cannot be started",
        }),
      ],
    });
    renderPage();

    const alerts = await screen.findAllByRole("alert");
    expect(alerts).toHaveLength(2);
    for (const alert of alerts) {
      expect(within(alert).getByRole("link", { name: "Open Runtimes" })).toHaveAttribute(
        "href",
        "/runtimes",
      );
    }
  });

  it("says which action failed", async () => {
    backendWith({
      list: () => [runningCaddy],
      jobs: () => [
        failed("caddy", { code: "SERVICE_STOP_FAILED", message: "caddy would not stop" }, "Stop"),
      ],
    });
    renderPage();

    expect(await screen.findByText("Caddy could not be stopped")).toBeInTheDocument();
    expect(screen.getByText("caddy would not stop")).toBeInTheDocument();
  });

  it("keeps a refused command on screen with the backend's reason, until it is dismissed", async () => {
    backendWith({
      list: () => [caddyRow],
      more: {
        service_start: () =>
          backendError(
            "BUSY",
            "lunaRun is quitting and is stopping its services: nothing can be started now",
          ),
      },
    });
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Start Caddy" }));

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Caddy did not start")).toBeInTheDocument();
    expect(within(alert).getByText(/lunaRun is quitting/)).toBeInTheDocument();
    // The button is usable again, so the user can try once more.
    expect(screen.getByRole("button", { name: "Start Caddy" })).toBeEnabled();

    await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("clears a refused command's message when the next attempt is made", async () => {
    let refuse = true;
    backendWith({
      list: () => [caddyRow],
      more: {
        service_start: () =>
          refuse
            ? backendError("BUSY", "caddy is busy with another action: wait for it to finish")
            : { id: "job-1", alreadyRunning: false },
      },
    });
    const user = userEvent.setup();
    renderPage();
    await user.click(await screen.findByRole("button", { name: "Start Caddy" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();

    refuse = false;
    await user.click(screen.getByRole("button", { name: "Start Caddy" }));

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  it("dismisses a failed job's message, and the next attempt replaces it", async () => {
    let jobs: JobInfo[] = [
      failed("caddy", {
        code: "SERVICE_START_FAILED",
        message: "caddy exited with code 1",
      }),
    ];
    backendWith({
      list: () => [{ ...caddyRow, state: "error" }],
      jobs: () => jobs,
      more: {
        service_start: () => {
          jobs = [serviceJob("caddy", "Start")];
          return { id: "job-start-caddy", alreadyRunning: false };
        },
      },
    });
    const user = userEvent.setup();
    renderPage();
    expect(await screen.findByRole("alert")).toBeInTheDocument();

    // Trying again makes the old message go away: the new attempt is what the page shows.
    await user.click(screen.getByRole("button", { name: "Start Caddy" }));

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
    expect(await screen.findByRole("button", { name: "Cancel starting Caddy" })).toBeEnabled();
  });
});
