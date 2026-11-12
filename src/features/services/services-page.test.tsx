import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SERVICE_STATE_EVENT } from "@/lib/events";
import { backendError, callsTo, emitEvent, listenerCount, resetBackend } from "@/test/backend";
import {
  backendWith,
  caddyRow,
  leftover,
  mysqlRow,
  phpFpm,
  phpPool,
  renderPage,
  row,
  status,
} from "@/test/services-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

beforeEach(() => {
  resetBackend();
});

/** The table row that mentions `name`. */
async function rowOf(name: string): Promise<HTMLElement> {
  const table = await screen.findByRole("region", { name: "Services" });
  const cell = within(table).getByText(name);
  const tableRow = cell.closest("tr");
  if (!tableRow) throw new Error(`no table row for ${name}`);
  return tableRow;
}

describe("ServicesPage", () => {
  it("shows a loading state first, then the services", async () => {
    backendWith();
    renderPage();
    expect(screen.getByRole("status", { name: "Loading services" })).toBeInTheDocument();

    expect(await screen.findByRole("region", { name: "Services" })).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "Loading services" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Services", level: 1 })).toBeInTheDocument();
  });

  it("lists each service with what it runs from, its ports and its state", async () => {
    backendWith({
      list: () => [
        caddyRow,
        phpPool,
        { ...mysqlRow, state: "running", running: 1, ports: [3307], needsSetup: false },
      ],
    });
    renderPage();

    const caddy = await rowOf("Caddy");
    expect(within(caddy).getByText("Version 2.11.7")).toBeInTheDocument();
    expect(within(caddy).getByText("Not assigned yet")).toBeInTheDocument();
    expect(within(caddy).getByText("Stopped")).toBeInTheDocument();

    const php = await rowOf("PHP 8.3.36");
    expect(within(php).getByText("4 workers")).toBeInTheDocument();
    expect(within(php).getByText("9100–9103")).toBeInTheDocument();
    expect(within(php).getByText("Starting")).toBeInTheDocument();
    expect(within(php).getByText("3 of 4 running")).toBeInTheDocument();

    const mysql = await rowOf("MySQL 8.4");
    expect(within(mysql).getByText("Version 8.4.13")).toBeInTheDocument();
    expect(within(mysql).getByText("3307")).toBeInTheDocument();
    expect(within(mysql).getByText("Running")).toBeInTheDocument();
  });

  it("gives the count of a pool only while some of it is not running", async () => {
    backendWith({
      list: () => [
        {
          ...phpPool,
          state: "running",
          running: 4,
          members: phpPool.members.map((member) => ({ ...member, state: "running" as const })),
        },
      ],
    });
    renderPage();

    const php = await rowOf("PHP 8.3.36");
    expect(within(php).getByText("Running")).toBeInTheDocument();
    expect(within(php).queryByText(/of 4 running/)).not.toBeInTheDocument();
  });

  it("shows every service, however many there are", async () => {
    const many = Array.from({ length: 12 }, (_, n) =>
      row(`php-fpm@8.${String(n)}.1`, { kind: "php", runtime: "php", version: `8.${String(n)}.1` }),
    );
    backendWith({ list: () => many });
    renderPage();

    const table = await screen.findByRole("region", { name: "Services" });
    expect(within(table).getAllByRole("row")).toHaveLength(13);
    // The table is not a short scroll box: the page scrolls, so no row is hidden inside it.
    expect(table).toHaveClass("max-h-none");
  });

  it("names PHP-FPM where it is a single process", async () => {
    backendWith({ list: () => [phpFpm] });
    renderPage();

    const php = await rowOf("PHP 8.4.12");
    expect(within(php).getByText("PHP-FPM")).toBeInTheDocument();
    expect(within(php).getByText("9100")).toBeInTheDocument();
    expect(within(php).queryByText(/running/)).not.toBeInTheDocument();
  });

  it("tells a stopped database without data that its first start sets it up", async () => {
    backendWith({ list: () => [mysqlRow] });
    renderPage();

    const mysql = await rowOf("MySQL 8.4");
    expect(within(mysql).getByText(/Its first start sets up its data folder/)).toBeInTheDocument();
  });

  it("does not say that about a database that has data, or a service that is not a database", async () => {
    backendWith({ list: () => [{ ...mysqlRow, needsSetup: false }, caddyRow] });
    renderPage();

    await rowOf("Caddy");
    expect(screen.queryByText(/first start sets up/)).not.toBeInTheDocument();
  });

  it("says that a service was ended because it did not stop in time", async () => {
    backendWith({
      list: () => [
        { ...mysqlRow, needsSetup: false, members: [status("mysql@8.4", { killed: true })] },
      ],
    });
    renderPage();

    const mysql = await rowOf("MySQL 8.4");
    expect(within(mysql).getByText(/did not stop in time and was ended/)).toBeInTheDocument();
    expect(within(mysql).getByText(/recovers by itself/)).toBeInTheDocument();
  });

  it("shows why a service crashed or failed to start, and that restarts ran out", async () => {
    backendWith({
      list: () => [
        row("mariadb@12.3", {
          kind: "database",
          runtime: "mariadb",
          version: "12.3.3",
          line: "12.3",
          state: "crashed",
          members: [
            status("mariadb@12.3", {
              state: "crashed",
              gaveUp: true,
              error: { code: "SERVICE_START_FAILED", message: "mariadbd exited with code 1" },
            }),
          ],
        }),
        row("caddy", {
          kind: "web",
          runtime: "caddy",
          version: "2.11.7",
          state: "error",
          members: [
            status("caddy", {
              state: "error",
              error: { code: "PORT_IN_USE", message: "port 8100 is already in use by httpd.exe" },
            }),
          ],
        }),
      ],
    });
    renderPage();

    const mariadb = await rowOf("MariaDB 12.3");
    expect(within(mariadb).getByText("Crashed")).toBeInTheDocument();
    expect(within(mariadb).getByText("mariadbd exited with code 1")).toBeInTheDocument();
    expect(within(mariadb).getByText(/stopped restarting it/)).toBeInTheDocument();
    const caddy = await rowOf("Caddy");
    expect(within(caddy).getByText("Failed to start")).toBeInTheDocument();
    expect(within(caddy).getByText("port 8100 is already in use by httpd.exe")).toBeInTheDocument();
  });

  it("says plainly that databases have no password and listen on this computer only", async () => {
    backendWith();
    renderPage();

    expect(await screen.findByText(/user root with no password/)).toBeInTheDocument();
    expect(screen.getByText(/this computer only/)).toBeInTheDocument();
  });

  it("does not mention databases when there is none", async () => {
    backendWith({ list: () => [caddyRow] });
    renderPage();

    await rowOf("Caddy");
    expect(screen.queryByText(/no password/)).not.toBeInTheDocument();
  });

  it("says honestly that starting and stopping is not here yet", async () => {
    backendWith();
    renderPage();

    expect(
      await screen.findByText(/Starting and stopping services comes in the next update/),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: /start|stop|restart/i })).not.toBeInTheDocument();
  });

  it("shows an empty state with the one action that fixes it when nothing is installed", async () => {
    backendWith({ list: () => [] });
    renderPage();

    expect(await screen.findByText("No services yet")).toBeInTheDocument();
    const action = screen.getByRole("link", { name: "Install a runtime" });
    expect(action).toHaveAttribute("href", "/runtimes");
    expect(screen.queryByRole("region", { name: "Services" })).not.toBeInTheDocument();
  });

  it("shows what failed and a way to try again, and recovers when it works", async () => {
    let attempt = 0;
    backendWith({
      list: () => {
        attempt += 1;
        return attempt === 1
          ? backendError("IO", "C:\\lüna home\\config\\services.json: access denied")
          : [caddyRow];
      },
    });
    renderPage();

    expect(await screen.findByText("Could not load the services")).toBeInTheDocument();
    expect(screen.getByText(/access denied/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));

    expect(await rowOf("Caddy")).toBeInTheDocument();
    expect(screen.queryByText("Could not load the services")).not.toBeInTheDocument();
  });

  it("asks the list again when a service changes state, and shows the new state", async () => {
    let running = false;
    backendWith({
      list: () => [
        {
          ...caddyRow,
          state: running ? "running" : "stopped",
          running: running ? 1 : 0,
          ports: running ? [2119] : [],
        },
      ],
    });
    renderPage();
    const caddy = await rowOf("Caddy");
    expect(within(caddy).getByText("Stopped")).toBeInTheDocument();

    running = true;
    emitEvent(SERVICE_STATE_EVENT, status("caddy", { state: "running", pid: 99 }));

    await waitFor(() => {
      expect(within(caddy).getByText("Running")).toBeInTheDocument();
    });
    expect(within(caddy).getByText("2119")).toBeInTheDocument();
    expect(callsTo("services_list").length).toBeGreaterThanOrEqual(2);
  });

  it("stops listening for state changes when the page goes away", async () => {
    backendWith();
    const { unmount } = renderPage();
    await rowOf("Caddy");
    expect(listenerCount(SERVICE_STATE_EVENT)).toBe(1);

    unmount();

    expect(listenerCount(SERVICE_STATE_EVENT)).toBe(0);
  });

  describe("services left from an earlier session", () => {
    it("lists them in a notice, without offering to do anything yet", async () => {
      backendWith({ leftovers: () => [leftover] });
      renderPage();

      const notice = await screen.findByText("Services from an earlier session are still running");
      expect(notice).toBeInTheDocument();
      expect(screen.getByText("mysql@8.4 (process 4242)")).toBeInTheDocument();
      expect(screen.getByText(/comes in the next update; until then/)).toBeInTheDocument();
    });

    it("shows no notice when nothing was left", async () => {
      backendWith();
      renderPage();

      await rowOf("Caddy");
      expect(screen.queryByText(/earlier session/)).not.toBeInTheDocument();
    });

    it("does not break the page when the list of them cannot be read", async () => {
      backendWith({
        leftovers: () => backendError("INTERNAL", "could not read the run folder"),
      });
      renderPage();

      expect(await rowOf("Caddy")).toBeInTheDocument();
      expect(screen.queryByText(/earlier session/)).not.toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });
});
