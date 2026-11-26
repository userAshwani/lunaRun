import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PortChange } from "@/bindings/PortChange";
import { backendError, callsTo, resetBackend } from "@/test/backend";
import { backendWith, detection, renderList, shop } from "@/test/projects-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/lib/native", () => ({
  pickFolder: vi.fn(),
  listenForFolderDrops: vi.fn(() => Promise.resolve(() => undefined)),
}));

beforeEach(() => {
  resetBackend();
});

/** A project that expects MySQL, installed, with a `.env` that says port 3307 and names the database `shop`. */
function withDatabase(file = ".env", database: string | null = "shop") {
  return () =>
    detection(shop, {
      report: {
        database: {
          engine: "mysql",
          connection: "mysql",
          host: "127.0.0.1",
          port: 3307,
          database,
          username: "root",
          file,
        },
      },
      resolution: {
        database: {
          engine: "mysql",
          runtime: "mysql",
          line: "8.4",
          installedVersion: "8.4.3",
          note: "none",
        },
      },
    });
}

const CHANGE: PortChange = {
  port: 3306,
  line: 4,
  before: "DB_PORT=3307",
  after: "DB_PORT=3306",
  backup: ".env.lunarun-backup",
};

async function databaseRow(): Promise<HTMLElement> {
  renderList("/projects/shop");
  const title = await screen.findByText("Database: MySQL 8.4 is installed (version 8.4.3)");
  const item = title.closest("li");
  if (!item) throw new Error("no database row");
  return item;
}

describe("the fixes of the database row", () => {
  describe("DB_PORT in .env", () => {
    it("shows nothing when the port is right", async () => {
      backendWith({ detect: withDatabase() });
      const row = await databaseRow();
      await waitFor(() => {
        expect(callsTo("project_env_port_preview")).toHaveLength(1);
      });
      expect(within(row).queryByRole("button", { name: /Update DB_PORT/ })).not.toBeInTheDocument();
    });

    it("never offers an edit of the example file", async () => {
      backendWith({
        detect: withDatabase(".env.example"),
        more: { project_env_port_preview: () => CHANGE },
      });
      const row = await databaseRow();
      expect(row).toHaveTextContent("Read from .env.example");
      expect(within(row).queryByRole("button", { name: /Update DB_PORT/ })).not.toBeInTheDocument();
      expect(callsTo("project_env_port_preview")).toHaveLength(0);
    });

    it("shows exactly the line that changes and the backup, and writes only after the confirmation", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: withDatabase(),
        more: {
          project_env_port_preview: () => CHANGE,
          project_env_port_apply: () => ({ backup: ".env.lunarun-backup" }),
        },
      });
      const row = await databaseRow();
      expect(
        await within(row).findByText(/does not connect to the database lunaRun runs/),
      ).toHaveTextContent("port 3306");

      await user.click(within(row).getByRole("button", { name: "Update DB_PORT in .env" }));

      const dialog = await screen.findByRole("dialog", { name: "Update DB_PORT in .env?" });
      expect(dialog).toHaveTextContent("Only this line changes");
      expect(dialog).toHaveTextContent(".env.lunarun-backup");
      const diff = within(dialog).getByRole("group", { name: "What changes in .env" });
      expect(diff).toHaveTextContent(".env, line 4");
      expect(within(diff).getByText("DB_PORT=3307")).toBeInTheDocument();
      expect(within(diff).getByText("DB_PORT=3306")).toBeInTheDocument();
      expect(within(diff).getByText("Removed line")).toBeInTheDocument();
      expect(within(diff).getByText("New line")).toBeInTheDocument();
      expect(
        callsTo("project_env_port_apply"),
        "nothing is written before the confirmation",
      ).toEqual([]);

      await user.click(within(dialog).getByRole("button", { name: "Update .env" }));

      await waitFor(() => {
        expect(callsTo("project_env_port_apply")).toEqual([[{ id: "shop", change: CHANGE }]]);
      });
      expect(
        await screen.findByText(/DB_PORT updated\. The old \.env is saved as/),
      ).toBeInTheDocument();
      await waitFor(() => {
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      });
    });

    it("cancelling writes nothing", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: withDatabase(),
        more: { project_env_port_preview: () => CHANGE },
      });
      const row = await databaseRow();
      await user.click(await within(row).findByRole("button", { name: "Update DB_PORT in .env" }));
      await user.click(
        within(await screen.findByRole("dialog")).getByRole("button", { name: "Cancel" }),
      );

      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(callsTo("project_env_port_apply")).toEqual([]);
    });

    it("says when the line is added because the file has no DB_PORT", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: withDatabase(),
        more: {
          project_env_port_preview: () => ({ ...CHANGE, before: null, line: 3 }),
        },
      });
      const row = await databaseRow();
      expect(await within(row).findByText(/has no DB_PORT/)).toHaveTextContent("port 3306");
      await user.click(within(row).getByRole("button", { name: "Update DB_PORT in .env" }));
      const diff = within(await screen.findByRole("dialog")).getByRole("group", {
        name: "What changes in .env",
      });
      expect(within(diff).queryByText("Removed line")).not.toBeInTheDocument();
      expect(within(diff).getByText("New line")).toBeInTheDocument();
    });

    it("explains a file that changed after the preview, and shows the problem next to the row", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: withDatabase(),
        more: {
          project_env_port_preview: () => CHANGE,
          project_env_port_apply: () =>
            backendError("ENV_CHANGED", "D:\\Work\\shop\\.env changed since the preview", {
              reason: "changed",
            }),
        },
      });
      const row = await databaseRow();
      await user.click(await within(row).findByRole("button", { name: "Update DB_PORT in .env" }));
      await user.click(
        within(await screen.findByRole("dialog")).getByRole("button", { name: "Update .env" }),
      );

      const alert = await within(row).findByRole("alert");
      expect(alert).toHaveTextContent("The .env file was not changed");
      expect(alert).toHaveTextContent("is not what the preview showed, so nothing was written");
      expect(alert).toHaveTextContent("changed since the preview");
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("says why a file that cannot be edited safely is not edited", async () => {
      backendWith({
        detect: withDatabase(),
        more: {
          project_env_port_preview: () =>
            backendError("ENV_CHANGED", "the .env is not UTF-8", { reason: "notUtf8" }),
        },
      });
      const row = await databaseRow();

      const alert = await within(row).findByRole("alert");
      expect(alert).toHaveTextContent("Could not check the .env file");
      expect(alert).toHaveTextContent("is not UTF-8 text, so lunaRun does not edit it");
      expect(within(alert).getByRole("button", { name: "Try again" })).toBeInTheDocument();
    });
  });

  describe("Create database", () => {
    it("offers it when the database is missing, creates it and says so", async () => {
      const user = userEvent.setup();
      let status = "missing";
      backendWith({
        detect: withDatabase(),
        more: {
          project_database_status: () => status,
          project_database_create: () => {
            status = "exists";
            return { name: "shop", created: true };
          },
        },
      });
      const row = await databaseRow();
      expect(await within(row).findByText("The database shop does not exist yet.")).toBeVisible();

      await user.click(within(row).getByRole("button", { name: "Create database" }));

      await waitFor(() => {
        expect(callsTo("project_database_create")).toEqual([[{ id: "shop" }]]);
      });
      expect(await screen.findByText("Created the database shop")).toBeInTheDocument();
      expect(await within(row).findByText("The database shop exists.")).toBeVisible();
      expect(
        within(row).queryByRole("button", { name: "Create database" }),
      ).not.toBeInTheDocument();
    });

    it("says when it was there already", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: withDatabase(),
        more: {
          project_database_status: () => "missing",
          project_database_create: () => ({ name: "shop", created: false }),
        },
      });
      const row = await databaseRow();
      await user.click(await within(row).findByRole("button", { name: "Create database" }));
      expect(await screen.findByText("The database shop was already there")).toBeInTheDocument();
    });

    it("says the server is not running, with the way out, when it is not", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: withDatabase(),
        more: {
          project_database_status: () => "missing",
          project_database_create: () =>
            backendError("DATABASE_NOT_RUNNING", "mysql@8.4 is not running", {
              service: "mysql@8.4",
            }),
        },
      });
      const row = await databaseRow();
      await user.click(await within(row).findByRole("button", { name: "Create database" }));

      const alert = await within(row).findByRole("alert");
      expect(alert).toHaveTextContent("Could not create the database");
      expect(alert).toHaveTextContent(
        "The database server is not running. Start the project first.",
      );
    });

    it("says the server has to run before it can be asked", async () => {
      backendWith({
        detect: withDatabase(),
        more: { project_database_status: () => "notRunning" },
      });
      const row = await databaseRow();
      expect(
        await within(row).findByText(
          "Start the project to check whether the database shop exists.",
        ),
      ).toBeVisible();
      expect(
        within(row).queryByRole("button", { name: "Create database" }),
      ).not.toBeInTheDocument();
    });

    it("refuses to create a database whose name lunaRun will not put into a statement", async () => {
      backendWith({
        detect: withDatabase(".env", "café shop"),
        more: { project_database_status: () => "invalidName" },
      });
      const row = await databaseRow();
      expect(await within(row).findByText(/The name café shop has characters/)).toBeVisible();
      expect(
        within(row).queryByRole("button", { name: "Create database" }),
      ).not.toBeInTheDocument();
    });

    it("shows a failed check with a way to try again", async () => {
      const user = userEvent.setup();
      let fail = true;
      backendWith({
        detect: withDatabase(),
        more: {
          project_database_status: () =>
            fail
              ? backendError("DATABASE_COMMAND_FAILED", "the database client exited with 1", {
                  tail: "ERROR 2002",
                })
              : "exists",
        },
      });
      const row = await databaseRow();
      const alert = await within(row).findByRole("alert");
      expect(alert).toHaveTextContent("Could not check the database");
      expect(alert).toHaveTextContent("The database did not accept the command");

      fail = false;
      await user.click(within(alert).getByRole("button", { name: "Try again" }));
      expect(await within(row).findByText("The database shop exists.")).toBeVisible();
    });

    it("shows nothing for a project that names no database", async () => {
      backendWith({
        detect: withDatabase(".env", null),
        more: { project_database_status: () => "noName" },
      });
      const row = await databaseRow();
      expect(callsTo("project_database_status")).toHaveLength(0);
      expect(row).not.toHaveTextContent("does not exist");
    });
  });
});
