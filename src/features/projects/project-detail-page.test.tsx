import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "@/bindings/Project";
import { backendError, callsTo, resetBackend } from "@/test/backend";
import {
  backendWith,
  detection,
  installPhp,
  readyPhp,
  recommendation,
  renderList,
  requirement,
  shop,
} from "@/test/projects-fixtures";
import { runtimeJob } from "@/test/runtimes-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/lib/native", () => ({
  pickFolder: vi.fn(),
  listenForFolderDrops: vi.fn(() => Promise.resolve(() => undefined)),
}));

beforeEach(() => {
  resetBackend();
});

async function openShop() {
  renderList("/projects/shop");
  return screen.findByRole("heading", { name: "Shop", level: 1 });
}

/** The checklist row whose title contains `text`. */
async function checkRow(text: string | RegExp): Promise<HTMLElement> {
  const title = await screen.findByText(text);
  const item = title.closest("li");
  if (!item) throw new Error(`no checklist row for ${String(text)}`);
  return item;
}

describe("ProjectDetailPage", () => {
  it("shows a loading state, then what the project needs", async () => {
    backendWith();
    renderList("/projects/shop");
    expect(screen.getByRole("status", { name: "Loading projects" })).toBeInTheDocument();

    expect(await screen.findByRole("heading", { name: "Shop", level: 1 })).toBeInTheDocument();
    // The folder is in the header and in "About this project".
    expect(screen.getAllByText("D:\\Work\\shop")).toHaveLength(2);
    expect(await screen.findByText("PHP 8.3.14 is installed")).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "Reading the project" })).not.toBeInTheDocument();
  });

  it("says why a version was chosen, and where the requirement was read", async () => {
    const user = userEvent.setup();
    backendWith();
    await openShop();

    const row = await checkRow("PHP 8.3.14 is installed");
    expect(row).toHaveTextContent("From the project files. Asks for >=8.2.0 <9.0.0");
    await user.click(within(row).getByText("Where this comes from"));
    expect(within(row).getByText("composer.json, require.php: ^8.2")).toBeVisible();
    expect(
      within(row).getByText("composer.lock, packages[symfony/console].require.php: >=8.2"),
    ).toBeVisible();
  });

  it("names what pinned a version, and other installed versions that would also do", async () => {
    backendWith({
      detect: () =>
        detection(shop, {
          report: {
            php: requirement(">=8.1.0 <8.2.0", {
              pin: { text: "8.1", exact: false, file: ".php-version" },
            }),
          },
          resolution: {
            php: readyPhp("8.1.30", {
              basis: "pinned",
              constraint: ">=8.1.0 <8.2.0",
              installedMatches: ["8.1.30", "8.1.2"],
            }),
          },
        }),
    });
    await openShop();

    const row = await checkRow("PHP 8.1.30 is installed");
    expect(row).toHaveTextContent("Pinned by .php-version.");
    expect(row).toHaveTextContent("Also installed and fitting: 8.1.2");
  });

  describe("a version that has to be installed", () => {
    const needsInstall = () => detection(shop, { resolution: { php: installPhp("8.3.36") } });

    it("offers to install it, naming the version and the project", async () => {
      const user = userEvent.setup();
      backendWith({
        detect: needsInstall,
        more: { runtime_install: () => ({ id: "job-1", alreadyRunning: false }) },
      });
      await openShop();

      const row = await checkRow("PHP 8.3.36 is needed and not installed yet");
      await user.click(within(row).getByRole("button", { name: "Install PHP 8.3.36 for Shop" }));

      expect(callsTo("runtime_install")).toEqual([[{ runtime: "php", version: "8.3.36" }]]);
    });

    it("shows the install's progress with Cancel instead of the button while it runs", async () => {
      backendWith({
        detect: needsInstall,
        jobs: () => [
          runtimeJob("php", "8.3.36", { phase: "downloading", bytesDone: 5, bytesTotal: 10 }),
        ],
      });
      await openShop();

      const row = await checkRow("PHP 8.3.36 is needed and not installed yet");
      expect(
        await within(row).findByRole("button", { name: "Cancel the installation of PHP 8.3.36" }),
      ).toBeInTheDocument();
      expect(within(row).queryByRole("button", { name: /Install PHP/ })).not.toBeInTheDocument();
    });

    it("says what went wrong when the install failed, and offers to try again", async () => {
      backendWith({
        detect: needsInstall,
        jobs: () => [
          runtimeJob("php", "8.3.36", {
            state: "failed",
            error: { code: "NETWORK", message: "connection reset" },
          }),
        ],
      });
      await openShop();

      const row = await checkRow("PHP 8.3.36 is needed and not installed yet");
      expect(await within(row).findByText("connection reset")).toBeInTheDocument();
      expect(within(row).getByRole("button", { name: /Try again/ })).toBeEnabled();
    });
  });

  it("explains when no version fits, including a line that exists but has no download here", async () => {
    backendWith({
      detect: () =>
        detection(shop, {
          resolution: {
            php: readyPhp("8.3.14", {
              recommended: null,
              installedMatches: [],
              constraint: ">=7.4.0 <8.0.0",
              issues: [
                {
                  code: "NO_COMPATIBLE_VERSION",
                  message: "english",
                  unavailableLines: ["7.4"],
                },
              ],
            }),
          },
        }),
    });
    await openShop();

    const row = await checkRow("No PHP version fits this project");
    expect(row).toHaveTextContent(
      "PHP 7.4 would fit, but there is no download of it for this computer.",
    );
    expect(row).toHaveTextContent("Asks for >=7.4.0 <8.0.0");
  });

  it("warns about a line past its end of life, a Rosetta build, and a choice the project does not allow", async () => {
    backendWith({
      detect: () =>
        detection(shop, {
          resolution: {
            php: readyPhp("8.1.30", {
              basis: "manual",
              conflictsWith: ">=8.3.0 <9.0.0",
              recommended: recommendation("8.1.30", { eol: true, rosettaFallback: true }),
            }),
          },
        }),
    });
    await openShop();

    const row = await checkRow("PHP 8.1.30 is installed");
    expect(row).toHaveTextContent("You chose this version for this project.");
    expect(row).toHaveTextContent("It asks for >=8.3.0 <9.0.0. It may not run.");
    expect(row).toHaveTextContent("no longer receives security fixes");
    expect(row).toHaveTextContent("runs under Rosetta");
    expect(within(row).getByText(/Needs attention/)).toBeInTheDocument();
  });

  describe("extensions", () => {
    it("lists the ones lunaRun's PHP does not have, and who needs them", async () => {
      backendWith({
        detect: () =>
          detection(shop, {
            resolution: {
              extensions: [
                {
                  name: "intl",
                  iniName: "intl",
                  neededBy: ["filament/filament"],
                  availability: "absent",
                },
                {
                  name: "mbstring",
                  iniName: "mbstring",
                  neededBy: ["Laravel"],
                  availability: "enabled",
                },
              ],
            },
          }),
      });
      await openShop();

      const row = await checkRow("PHP extensions: 1 required extension is not in lunaRun's PHP");
      expect(row).toHaveTextContent("intl: Not available. Needed by filament/filament");
      expect(row).toHaveTextContent("Enabling extensions comes in a later version");
      expect(row).not.toHaveTextContent("mbstring");
    });

    it("says so when none is known to be missing, and when the project names none", async () => {
      backendWith({
        detect: () =>
          detection(shop, {
            resolution: {
              extensions: [
                {
                  name: "mbstring",
                  iniName: "mbstring",
                  neededBy: ["Laravel"],
                  availability: "enabled",
                },
                {
                  name: "imaginary",
                  iniName: "imaginary",
                  neededBy: ["x/y"],
                  availability: "unknown",
                },
              ],
            },
          }),
      });
      await openShop();
      expect(
        await screen.findByText(
          "PHP extensions: None of the 2 required extensions is known to be missing",
        ),
      ).toBeInTheDocument();
    });

    it("has a row for a project that names no extension", async () => {
      backendWith();
      await openShop();
      expect(
        await screen.findByText("The project does not name any PHP extensions"),
      ).toBeInTheDocument();
    });
  });

  it("shows Node.js when the project uses it, and not when it does not", async () => {
    backendWith({
      detect: () =>
        detection(shop, {
          report: { node: requirement("lts/*") },
          resolution: { node: { ...readyPhp("22.11.0"), runtime: "node", constraint: "lts/*" } },
        }),
    });
    await openShop();
    expect(await screen.findByText("Node.js 22.11.0 is installed")).toBeInTheDocument();
  });

  it("does not show a Node.js row for a project without one", async () => {
    backendWith();
    await openShop();
    await screen.findByText("PHP 8.3.14 is installed");
    expect(screen.queryByText(/Node\.js .* is installed/)).not.toBeInTheDocument();
  });

  describe("database", () => {
    it("says which database the project expects and whether it is installed", async () => {
      backendWith({
        detect: () =>
          detection(shop, {
            report: {
              database: {
                engine: "mysql",
                connection: "mysql",
                host: "127.0.0.1",
                port: 3307,
                database: "shop",
                username: "root",
                file: ".env",
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
          }),
      });
      await openShop();

      const row = await checkRow("Database: MySQL 8.4 is installed (version 8.4.3)");
      expect(row).toHaveTextContent("The project expects it on port 3307.");
      expect(row).toHaveTextContent("Read from .env");
    });

    it("offers the Runtimes page when it is not installed, and explains what is not supported", async () => {
      backendWith({
        detect: () =>
          detection(shop, {
            resolution: {
              database: {
                engine: "mariadb",
                runtime: "mysql",
                line: "8.4",
                installedVersion: null,
                note: "mariadbNotOnMacos",
              },
            },
          }),
      });
      await openShop();

      const row = await checkRow("Database: MySQL 8.4 is not installed yet");
      expect(row).toHaveTextContent(
        "MariaDB is not offered on macOS, so MySQL is suggested instead.",
      );
      expect(within(row).getByRole("link", { name: "Runtimes" })).toHaveAttribute(
        "href",
        "/runtimes",
      );
    });

    it("says SQLite needs nothing", async () => {
      backendWith({
        detect: () =>
          detection(shop, {
            resolution: {
              database: {
                engine: "sqlite",
                runtime: null,
                line: null,
                installedVersion: null,
                note: "sqliteNeedsNothing",
              },
            },
          }),
      });
      await openShop();
      const row = await checkRow("Database: SQLite");
      expect(row).toHaveTextContent("Nothing needs installing.");
    });
  });

  it("lists what the scan could not read or had to decide, each in a sentence with the detail underneath", async () => {
    backendWith({
      detect: () =>
        detection(shop, {
          report: {
            warnings: [
              {
                code: "PIN_CONFLICTS_WITH_COMPOSER",
                file: ".php-version",
                message: "english",
                details: ["8.1", ">=8.2.0 <9.0.0"],
              },
              {
                code: "INVALID_JSON",
                file: "composer.json",
                message: "english",
                details: ["line 3, column 1"],
              },
            ],
          },
        }),
    });
    await openShop();

    expect(await screen.findByText("Things to look at")).toBeInTheDocument();
    expect(
      screen.getByText(
        ".php-version pins a PHP version that composer.json does not allow. The pin is used.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("8.1 · >=8.2.0 <9.0.0")).toBeInTheDocument();
    expect(
      screen.getByText("composer.json is not valid JSON, so it was skipped."),
    ).toBeInTheDocument();
  });

  it("claims nothing about a folder that has gone, and says what to do", async () => {
    backendWith({
      detect: () =>
        detection(shop, {
          report: {
            warnings: [
              { code: "PROJECT_FOLDER_MISSING", file: null, message: "gone", details: [] },
            ],
          },
        }),
    });
    await openShop();

    expect(
      await screen.findByText(/The folder is not there, so there is nothing to check/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/is installed/)).not.toBeInTheDocument();
    expect(screen.getByText(/The project folder is missing/)).toBeInTheDocument();
  });

  it("reads the folder again on Re-scan", async () => {
    const user = userEvent.setup();
    backendWith();
    await openShop();
    await screen.findByText("PHP 8.3.14 is installed");
    expect(callsTo("project_detect")).toHaveLength(1);

    await user.click(screen.getByRole("button", { name: "Re-scan Shop" }));

    await waitFor(() => {
      expect(callsTo("project_detect")).toHaveLength(2);
    });
  });

  it("shows why a project could not be read, and tries again", async () => {
    const user = userEvent.setup();
    let fail = true;
    backendWith({
      detect: () => (fail ? backendError("IO", "disk error") : detection(shop)),
    });
    await openShop();

    expect(await screen.findByText("Could not read this project")).toBeInTheDocument();
    expect(screen.getByText("disk error")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("PHP 8.3.14 is installed")).toBeInTheDocument();
  });

  it("says when the project is not in the list", async () => {
    backendWith();
    renderList("/projects/nothing");
    expect(await screen.findByText("This project is not in your list")).toBeInTheDocument();
    expect(screen.getAllByRole("link", { name: /All projects/ }).length).toBeGreaterThan(0);
  });

  it("shows what it knows about the project: folder, framework, document root and when it was read", async () => {
    backendWith();
    await openShop();
    const about = (await screen.findByText("About this project")).closest("div.rounded-xl");
    expect(about).not.toBeNull();
    const card = within(about as HTMLElement);
    expect(card.getByText("Laravel 11.9.0")).toBeInTheDocument();
    expect(card.getByText("public (Laravel serves from public/)")).toBeInTheDocument();
    expect(card.getByText("None detected")).toBeInTheDocument();
    expect(card.getByText(/Last scanned/)).toBeInTheDocument();
  });

  describe("settings", () => {
    /** Backend whose stored project follows `project_update`, as the real one does. */
    function settingsBackend(extra: { fail?: () => Promise<never> | undefined } = {}) {
      let stored: Project = shop;
      backendWith({
        projects: () => [stored],
        detect: () => detection(stored),
        more: {
          project_update: (args) => {
            const failure = extra.fail?.();
            if (failure) return failure;
            const { patch } = args as { patch: Record<string, unknown> };
            stored = {
              ...stored,
              ...(typeof patch.name === "string" ? { name: patch.name.trim() } : {}),
              ...(patch.docRoot
                ? {
                    docRoot: (patch.docRoot as { path?: string }).path ?? "public",
                    docRootCustom: true,
                  }
                : {}),
              bindings: {
                ...stored.bindings,
                ...(patch.php
                  ? { php: (patch.php as { binding?: Project["bindings"]["php"] }).binding ?? null }
                  : {}),
              },
            };
            return stored;
          },
        },
      });
    }

    it("renames the project", async () => {
      const user = userEvent.setup();
      settingsBackend();
      await openShop();

      const field = await screen.findByLabelText("Name");
      await user.clear(field);
      await user.type(field, "  Storefront ");
      await user.click(screen.getByRole("button", { name: "Save name" }));

      expect(
        await screen.findByRole("heading", { name: "Storefront", level: 1 }),
      ).toBeInTheDocument();
      expect(callsTo("project_update")).toEqual([
        [{ id: "shop", patch: { name: "  Storefront " } }],
      ]);
      expect(screen.getByText("Renamed to Storefront")).toBeInTheDocument();
    });

    it("does not offer to save an unchanged or empty name", async () => {
      const user = userEvent.setup();
      settingsBackend();
      await openShop();

      const field = await screen.findByLabelText("Name");
      expect(screen.getByRole("button", { name: "Save name" })).toBeDisabled();
      await user.clear(field);
      expect(screen.getByRole("button", { name: "Save name" })).toBeDisabled();
    });

    it("changes the document root, and can go back to the detected one", async () => {
      const user = userEvent.setup();
      settingsBackend();
      await openShop();

      const field = await screen.findByLabelText("Document root");
      await user.clear(field);
      await user.type(field, "web");
      await user.click(screen.getByRole("button", { name: "Save document root" }));

      expect(await screen.findByText(/You chose this folder\./)).toBeInTheDocument();
      expect(callsTo("project_update")[0]).toEqual([
        { id: "shop", patch: { docRoot: { kind: "use", path: "web" } } },
      ]);
      await user.click(screen.getByRole("button", { name: "Use the detected one" }));
      await waitFor(() => {
        expect(callsTo("project_update")).toHaveLength(2);
      });
      expect(callsTo("project_update")[1]).toEqual([
        { id: "shop", patch: { docRoot: { kind: "auto" } } },
      ]);
    });

    it("chooses a PHP version by line or exact version, or goes back to automatic", async () => {
      const user = userEvent.setup();
      settingsBackend();
      await openShop();

      const select = await screen.findByLabelText("PHP version");
      expect(select).toHaveValue("");
      expect(
        within(select).getByRole("option", { name: "Automatic: follow the project" }),
      ).toBeInTheDocument();
      expect(
        within(select).getByRole("option", { name: "PHP 8.4 (newest patch)" }),
      ).toBeInTheDocument();
      // 9.0.0 has no download for this computer: not offered.
      expect(within(select).queryByRole("option", { name: /PHP 9\.0/ })).not.toBeInTheDocument();

      await user.selectOptions(select, "PHP 8.1 (newest patch)");
      await waitFor(() => {
        expect(callsTo("project_update")).toHaveLength(1);
      });
      expect(callsTo("project_update")[0]).toEqual([
        { id: "shop", patch: { php: { kind: "use", binding: { mode: "line", value: "8.1" } } } },
      ]);
      expect(
        await screen.findByText("PHP for this project is now PHP 8.1 (newest patch)"),
      ).toBeInTheDocument();

      await user.selectOptions(select, "PHP 8.3.9 (this exact version)");
      await waitFor(() => {
        expect(callsTo("project_update")).toHaveLength(2);
      });
      expect(callsTo("project_update")[1]).toEqual([
        { id: "shop", patch: { php: { kind: "use", binding: { mode: "exact", value: "8.3.9" } } } },
      ]);

      await user.selectOptions(select, "Automatic: follow the project");
      await waitFor(() => {
        expect(callsTo("project_update")).toHaveLength(3);
      });
      expect(callsTo("project_update")[2]).toEqual([
        { id: "shop", patch: { php: { kind: "auto" } } },
      ]);
    });

    it("shows what the backend refused, keeping the technical message", async () => {
      const user = userEvent.setup();
      settingsBackend({
        fail: () => backendError("INVALID_INPUT", "the project has no folder web"),
      });
      await openShop();

      const field = await screen.findByLabelText("Document root");
      await user.clear(field);
      await user.type(field, "web");
      await user.click(screen.getByRole("button", { name: "Save document root" }));

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Could not save");
      expect(alert).toHaveTextContent("That value cannot be used");
      expect(alert).toHaveTextContent("the project has no folder web");
    });
  });

  describe("removing", () => {
    it("asks first, then goes back to the list", async () => {
      const user = userEvent.setup();
      let list = [shop];
      backendWith({
        projects: () => list,
        more: {
          project_remove: () => {
            list = [];
            return null;
          },
        },
      });
      await openShop();

      await user.click(await screen.findByRole("button", { name: "Remove Shop" }));
      const dialog = screen.getByRole("dialog");
      expect(dialog).toHaveTextContent("Remove Shop?");
      expect(dialog).toHaveTextContent("are not touched");
      await user.click(within(dialog).getByRole("button", { name: "Remove project" }));

      expect(await screen.findByRole("heading", { name: "No projects yet" })).toBeInTheDocument();
      expect(callsTo("project_remove")).toEqual([[{ id: "shop" }]]);
    });
  });
});
