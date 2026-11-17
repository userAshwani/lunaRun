import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DatabaseSettings } from "@/bindings/DatabaseSettings";
import type { JobInfo } from "@/bindings/JobInfo";
import { backendError, callsTo, resetBackend } from "@/test/backend";
import {
  backendWith,
  blog,
  detection,
  renderList,
  shop,
  siteEntry,
  sitesView,
} from "@/test/projects-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/lib/native", () => ({
  pickFolder: vi.fn(),
  listenForFolderDrops: vi.fn(() => Promise.resolve(() => undefined)),
}));

beforeEach(() => {
  resetBackend();
});

/** A project's start or stop job, as the backend lists it. */
function projectJob(action: "Start" | "Stop", extra: Partial<JobInfo> = {}): JobInfo {
  return {
    id: "job-p1",
    key: "project:shop",
    label: `${action} project Shop`,
    state: "running",
    phase: "starting",
    bytesDone: 0,
    bytesTotal: null,
    message: null,
    error: null,
    ...extra,
  };
}

const START_OK = { id: "job-p1", alreadyRunning: false };

async function openShop() {
  renderList("/projects/shop");
  await screen.findByRole("heading", { name: "Shop", level: 1 });
  return screen.findByRole("heading", { name: "Run this project" });
}

/** The Run card as a whole. */
async function card(): Promise<HTMLElement> {
  const title = await openShop();
  const element = title.closest("div.rounded-xl");
  if (!element) throw new Error("no Run card");
  return element as HTMLElement;
}

const SETTINGS: DatabaseSettings = {
  service: "mysql@8.4",
  runtime: "mysql",
  host: "127.0.0.1",
  port: 3307,
  database: "shop",
  username: "root",
};

describe("the Run card", () => {
  describe("the state", () => {
    it("says a project that was not started is stopped, and offers to start it", async () => {
      backendWith({ sites: () => sitesView([siteEntry("shop")]) });
      const run = within(await card());

      expect(await run.findByText("Stopped")).toBeInTheDocument();
      expect(run.getByText(/Starting brings up this project's PHP/)).toBeInTheDocument();
      expect(run.getByRole("button", { name: "Start Shop" })).toBeEnabled();
      expect(run.queryByRole("button", { name: "Stop Shop" })).not.toBeInTheDocument();
    });

    it("says a project is running and where it is served, and offers to stop it", async () => {
      backendWith({
        sites: () => sitesView([siteEntry("shop", { served: true, running: true })], "running"),
      });
      const run = within(await card());

      expect(await run.findByText("Running")).toBeInTheDocument();
      expect(run.getByText("Served at http://127.0.0.1:8101.")).toBeInTheDocument();
      expect(run.getByRole("button", { name: "Stop Shop" })).toBeEnabled();
      expect(run.queryByRole("button", { name: "Start Shop" })).not.toBeInTheDocument();
    });

    it("says so when the project was started but a service it needs is no longer running", async () => {
      backendWith({
        sites: () => sitesView([siteEntry("shop", { served: true, running: false })], "stopped"),
      });
      const run = within(await card());

      expect(await run.findByText("Not answering")).toBeInTheDocument();
      expect(run.getByText(/a service it needs is not running/)).toBeInTheDocument();
      expect(run.getByRole("button", { name: "Stop Shop" })).toBeEnabled();
    });

    it("cannot start a project that has no site, and says why", async () => {
      backendWith({
        sites: () =>
          sitesView([
            siteEntry("shop", { state: "noPhp", port: null, address: null, phpVersion: null }),
          ]),
      });
      const run = within(await card());

      expect(await run.findByText("Stopped")).toBeInTheDocument();
      expect(
        run.getByText(/No site yet: the PHP this project needs is not installed/),
      ).toBeInTheDocument();
      expect(run.getByRole("button", { name: "Start Shop" })).toBeDisabled();
    });

    it("shows a placeholder while the sites are being read", async () => {
      backendWith({ sites: () => new Promise(() => undefined) });
      const run = within(await card());
      expect(run.getByLabelText("Reading the site")).toBeInTheDocument();
      expect(run.getByRole("button", { name: "Start Shop" })).toBeDisabled();
    });
  });

  describe("starting and stopping", () => {
    it("starts the project", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        more: { project_start: () => START_OK },
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Start Shop" }));

      await waitFor(() => {
        expect(callsTo("project_start")).toEqual([[{ id: "shop" }]]);
      });
    });

    it("stops the project", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop", { served: true, running: true })], "running"),
        more: { project_stop: () => START_OK },
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Stop Shop" }));

      await waitFor(() => {
        expect(callsTo("project_stop")).toEqual([[{ id: "shop" }]]);
      });
    });

    it("shows the progress of a start with Cancel, and cancels it", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop", { served: true })]),
        jobs: () => [projectJob("Start", { phase: "setup" })],
        more: { job_cancel: () => true },
      });
      const run = within(await card());

      expect(await run.findByText("Starting")).toBeInTheDocument();
      expect(run.getByText("Bringing up what this project needs.")).toBeInTheDocument();
      expect(run.getByRole("progressbar", { name: "Starting Shop" })).toBeInTheDocument();
      await user.click(run.getByRole("button", { name: "Cancel starting Shop" }));

      await waitFor(() => {
        expect(callsTo("job_cancel")).toEqual([[{ jobId: "job-p1" }]]);
      });
    });

    it("shows a stop under way, with Stop off", async () => {
      backendWith({
        sites: () => sitesView([siteEntry("shop", { served: true, running: true })], "running"),
        jobs: () => [projectJob("Stop", { phase: "stopping" })],
      });
      const run = within(await card());

      // The badge and the phase of the job both say it.
      expect(await run.findAllByText("Stopping")).toHaveLength(2);
      expect(run.getByRole("button", { name: "Stop Shop" })).toBeDisabled();
      expect(run.queryByRole("button", { name: /Cancel/ })).not.toBeInTheDocument();
    });
  });

  describe("when it fails", () => {
    it("names the program that holds a port, and keeps the technical message", async () => {
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        jobs: () => [
          projectJob("Start", {
            state: "failed",
            error: {
              code: "PORT_IN_USE",
              message: "port 8101 is in use by httpd.exe",
              details: { port: 8101, processName: "httpd.exe" },
            },
          }),
        ],
      });
      const alert = await within(await card()).findByRole("alert");

      expect(alert).toHaveTextContent("Shop could not be started");
      expect(alert).toHaveTextContent("Port 8101 is used by httpd.exe.");
      expect(alert).toHaveTextContent("port 8101 is in use by httpd.exe");
    });

    it("says what to do when the PHP is not installed or the folder is gone", async () => {
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        jobs: () => [
          projectJob("Start", {
            state: "failed",
            error: { code: "NO_SITE", message: "x", details: { reason: "php" } },
          }),
        ],
      });
      const alert = await within(await card()).findByRole("alert");
      expect(alert).toHaveTextContent(
        "Its PHP is not installed. Install it from the checklist above",
      );
    });

    it("says it for a stop too, and the problem can be dismissed", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop", { served: true, running: true })], "running"),
        jobs: () => [
          projectJob("Stop", {
            state: "failed",
            error: { code: "RELOAD_FAILED", message: "Caddy could not reload" },
          }),
        ],
      });
      const alert = await within(await card()).findByRole("alert");
      expect(alert).toHaveTextContent("Shop could not be stopped");
      expect(alert).toHaveTextContent("The service could not load its new configuration");

      await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
      expect(screen.queryByText("Shop could not be stopped")).not.toBeInTheDocument();
    });

    it("shows what the backend refuses at once", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        more: { project_start: () => backendError("BUSY", "lunaRun is quitting") },
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Start Shop" }));

      const alert = await run.findByRole("alert");
      expect(alert).toHaveTextContent("Shop could not be started");
      expect(alert).toHaveTextContent("Another operation is running");
      expect(alert).toHaveTextContent("lunaRun is quitting");
    });
  });

  describe("the quick actions", () => {
    it("opens the site, the folder and the editor of the project", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        more: { project_open: () => null },
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Open in browser" }));
      await user.click(run.getByRole("button", { name: "Open folder" }));
      await user.click(run.getByRole("button", { name: "Open in editor" }));

      await waitFor(() => {
        expect(callsTo("project_open")).toEqual([
          [{ id: "shop", target: "browser" }],
          [{ id: "shop", target: "folder" }],
          [{ id: "shop", target: "editor" }],
        ]);
      });
    });

    it("cannot open a site that does not exist, but can open the folder", async () => {
      backendWith({
        sites: () =>
          sitesView([
            siteEntry("shop", { state: "noPhp", port: null, address: null, phpVersion: null }),
          ]),
      });
      const run = within(await card());

      expect(await run.findByRole("button", { name: "Open in browser" })).toBeDisabled();
      expect(run.getByRole("button", { name: "Open folder" })).toBeEnabled();
    });

    it("says plainly when Visual Studio Code is not installed", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        more: {
          project_open: () =>
            // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the backend rejects with a plain object
            Promise.reject({
              code: "NOT_FOUND",
              message: "Visual Studio Code was not found in the usual places",
              details: { target: "editor" },
            }),
        },
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Open in editor" }));

      const alert = await run.findByRole("alert");
      expect(alert).toHaveTextContent("That could not be opened");
      expect(alert).toHaveTextContent(
        "Visual Studio Code was not found in the usual places. Install it",
      );
      await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
      expect(run.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("names the category for anything else that cannot be opened", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        more: { project_open: () => backendError("NOT_FOUND", "the project folder is not there") },
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Open folder" }));

      const alert = await run.findByRole("alert");
      expect(alert).toHaveTextContent("Not found");
      expect(alert).toHaveTextContent("the project folder is not there");
    });

    it("copies the settings of the database lunaRun runs, without a password", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        database: () => SETTINGS,
      });
      const run = within(await card());

      await user.click(await run.findByRole("button", { name: "Copy database settings" }));

      expect(await run.findByText("Copied")).toBeInTheDocument();
      expect(await navigator.clipboard.readText()).toBe(
        "DB_CONNECTION=mysql\nDB_HOST=127.0.0.1\nDB_PORT=3307\nDB_DATABASE=shop\nDB_USERNAME=root\nDB_PASSWORD=\n",
      );
      expect(
        run.getByText(
          /Connects as root with no password on port 3307\. The password in your \.env file is never read\./,
        ),
      ).toBeInTheDocument();
    });

    it("leaves out the database name when the project does not name one, and offers nothing without a database", async () => {
      const user = userEvent.setup();
      backendWith({
        sites: () => sitesView([siteEntry("shop")]),
        database: () => ({ ...SETTINGS, database: null, runtime: "mariadb", port: 3308 }),
      });
      const run = within(await card());
      await user.click(await run.findByRole("button", { name: "Copy database settings" }));
      expect(await navigator.clipboard.readText()).toBe(
        "DB_CONNECTION=mariadb\nDB_HOST=127.0.0.1\nDB_PORT=3308\nDB_USERNAME=root\nDB_PASSWORD=\n",
      );
    });

    it("has no database button for a project that has no database here", async () => {
      backendWith({ sites: () => sitesView([siteEntry("shop")]) });
      const run = within(await card());
      expect(await run.findByRole("button", { name: "Open folder" })).toBeInTheDocument();
      expect(run.queryByRole("button", { name: "Copy database settings" })).not.toBeInTheDocument();
    });
  });
});

describe("the list", () => {
  it("shows Running on the project that is running and on no other", async () => {
    backendWith({
      sites: () =>
        sitesView(
          [
            siteEntry("shop", { served: true, running: true }),
            siteEntry("blog", {
              port: 8102,
              address: "http://127.0.0.1:8102",
              served: true,
              running: false,
            }),
          ],
          "running",
        ),
      detect: (id) => detection(id === "shop" ? shop : blog),
    });
    renderList("/projects");

    const table = await screen.findByRole("region", { name: "Projects" });
    const rowOf = (name: string) => {
      const row = within(table).getByRole("link", { name }).closest("tr");
      if (!row) throw new Error(name);
      return within(row);
    };
    await waitFor(() => {
      expect(rowOf("Shop").getByText("Running")).toBeInTheDocument();
    });
    expect(rowOf("Blog").queryByText("Running")).not.toBeInTheDocument();
  });
});
