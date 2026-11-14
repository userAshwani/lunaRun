import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "@/bindings/Project";
import type { FolderDropHandlers } from "@/lib/native";
import { listenForFolderDrops, pickFolder } from "@/lib/native";
import { backendError, callsTo, resetBackend } from "@/test/backend";
import {
  backendWith,
  blog,
  detection,
  installPhp,
  project,
  readyPhp,
  renderList,
  shop,
} from "@/test/projects-fixtures";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/lib/native", () => ({ pickFolder: vi.fn(), listenForFolderDrops: vi.fn() }));

let drop: FolderDropHandlers | null = null;

beforeEach(() => {
  resetBackend();
  drop = null;
  vi.mocked(pickFolder).mockReset();
  vi.mocked(listenForFolderDrops).mockReset();
  vi.mocked(listenForFolderDrops).mockImplementation((handlers) => {
    drop = handlers;
    return Promise.resolve(() => undefined);
  });
});

/** The table row that mentions `name`. */
async function rowOf(name: string): Promise<HTMLElement> {
  const table = await screen.findByRole("region", { name: "Projects" });
  const link = within(table).getByRole("link", { name });
  const tableRow = link.closest("tr");
  if (!tableRow) throw new Error(`no table row for ${name}`);
  return tableRow;
}

describe("ProjectsPage", () => {
  it("shows a loading state first, then the projects", async () => {
    backendWith();
    renderList();
    expect(screen.getByRole("status", { name: "Loading projects" })).toBeInTheDocument();

    expect(await screen.findByRole("region", { name: "Projects" })).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "Loading projects" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Projects", level: 1 })).toBeInTheDocument();
  });

  it("lists each project with its folder, framework and the PHP it will run on", async () => {
    backendWith({
      detect: (id) =>
        id === "shop"
          ? detection(shop)
          : detection(blog, { resolution: { php: installPhp("8.3.36") } }),
    });
    renderList();

    const shopRow = await rowOf("Shop");
    expect(within(shopRow).getByText("D:\\Work\\shop")).toBeInTheDocument();
    expect(within(shopRow).getByText(/Laravel 11\.9\.0/)).toBeInTheDocument();
    expect(await within(shopRow).findByText("PHP 8.3.14")).toBeInTheDocument();
    expect(within(shopRow).getByText("Not required")).toBeInTheDocument();

    const blogRow = await rowOf("Blog");
    expect(within(blogRow).getByText(/PHP project/)).toBeInTheDocument();
    expect(await within(blogRow).findByText("PHP 8.3.36")).toBeInTheDocument();
    expect(within(blogRow).getByText(". Needs install")).toBeInTheDocument();
  });

  it("says when nothing fits, and when a project's folder has gone", async () => {
    backendWith({
      projects: () => [shop, blog],
      detect: (id) =>
        id === "shop"
          ? detection(shop, {
              resolution: { php: readyPhp("8.3.14", { recommended: null, installedMatches: [] }) },
            })
          : detection(blog, {
              report: {
                warnings: [
                  { code: "PROJECT_FOLDER_MISSING", file: null, message: "gone", details: [] },
                ],
              },
            }),
    });
    renderList();

    expect(await within(await rowOf("Shop")).findByText("No match")).toBeInTheDocument();
    const gone = await rowOf("Blog");
    expect(await within(gone).findByText("Folder missing")).toBeInTheDocument();
    // Nothing is claimed about a folder that is not there.
    expect(within(gone).getAllByText("Not checked")).toHaveLength(2);
    expect(within(gone).queryByText(/PHP 8\./)).not.toBeInTheDocument();
  });

  it("shows a note instead of a version when one project cannot be read, and the rest still work", async () => {
    backendWith({
      detect: (id) => (id === "shop" ? backendError("IO", "cannot read") : detection(blog)),
    });
    renderList();

    expect(await within(await rowOf("Shop")).findAllByText("Could not read")).toHaveLength(2);
    expect(await within(await rowOf("Blog")).findByText("PHP 8.3.14")).toBeInTheDocument();
  });

  it("explains what a project is when there are none, and offers to add one", async () => {
    backendWith({ projects: () => [] });
    renderList();

    expect(await screen.findByRole("heading", { name: "No projects yet" })).toBeInTheDocument();
    expect(screen.getByText(/It never changes your files/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add project" })).toBeEnabled();
  });

  it("shows why the list could not be loaded, and tries again", async () => {
    const user = userEvent.setup();
    let fail = true;
    backendWith({
      projects: () => (fail ? backendError("IO", "projects.json is locked") : [shop]),
    });
    renderList();

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load your projects");
    expect(screen.getByText("projects.json is locked")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("region", { name: "Projects" })).toBeInTheDocument();
  });

  describe("adding", () => {
    it("adds the folder the picker returns and opens the new project", async () => {
      const user = userEvent.setup();
      vi.mocked(pickFolder).mockResolvedValue("D:\\Work\\mon projet é");
      const added = project("new1", { name: "mon projet é", path: "D:\\Work\\mon projet é" });
      let list: Project[] = [];
      backendWith({
        projects: () => list,
        detect: () => detection(added),
        more: {
          project_add: () => {
            list = [added];
            return detection(added);
          },
        },
      });
      renderList();

      await user.click(await screen.findByRole("button", { name: "Add project" }));

      expect(
        await screen.findByRole("heading", { name: "mon projet é", level: 1 }),
      ).toBeInTheDocument();
      expect(callsTo("project_add")).toEqual([[{ path: "D:\\Work\\mon projet é" }]]);
      expect(screen.getByText("Added mon projet é")).toBeInTheDocument();
    });

    it("does nothing when the picker is closed without a choice", async () => {
      const user = userEvent.setup();
      vi.mocked(pickFolder).mockResolvedValue(null);
      backendWith({ projects: () => [shop] });
      renderList();

      await user.click(await screen.findByRole("button", { name: "Add project" }));

      await waitFor(() => {
        expect(pickFolder).toHaveBeenCalledTimes(1);
      });
      expect(callsTo("project_add")).toEqual([]);
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("says a folder is already a project and links to it", async () => {
      const user = userEvent.setup();
      vi.mocked(pickFolder).mockResolvedValue("D:\\Work\\shop");
      backendWith({
        projects: () => [shop],
        more: {
          project_add: () => {
            // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the backend rejects with a plain object
            return Promise.reject({
              code: "PROJECT_EXISTS",
              message: "D:\\Work\\shop is already the project Shop",
              details: { id: "shop", name: "Shop" },
            });
          },
        },
      });
      renderList();

      await user.click(await screen.findByRole("button", { name: "Add project" }));

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Could not add that folder");
      expect(alert).toHaveTextContent("Shop is already in your projects.");
      expect(within(alert).getByRole("link", { name: "Open Shop" })).toHaveAttribute(
        "href",
        "/projects/shop",
      );
    });

    it("names the category and keeps the technical message when the folder is refused, and can be dismissed", async () => {
      const user = userEvent.setup();
      vi.mocked(pickFolder).mockResolvedValue("D:\\Work\\nothing");
      backendWith({
        projects: () => [shop],
        more: {
          project_add: () =>
            backendError("INVALID_PATH", "D:\\Work\\nothing is not a folder that exists"),
        },
      });
      renderList();

      await user.click(await screen.findByRole("button", { name: "Add project" }));

      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Invalid path");
      expect(alert).toHaveTextContent("D:\\Work\\nothing is not a folder that exists");
      await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("shows where to drop while a folder is dragged over the window, and adds what is dropped", async () => {
      backendWith({
        projects: () => [shop],
        more: {
          project_add: (args) => detection(project(`id-${(args as { path: string }).path}`)),
        },
      });
      renderList();
      await screen.findByRole("region", { name: "Projects" });
      await waitFor(() => {
        expect(drop).not.toBeNull();
      });

      drop?.onHover(true);
      expect(await screen.findByText("Drop a folder to add it as a project")).toBeInTheDocument();
      drop?.onHover(false);
      await waitFor(() => {
        expect(screen.queryByText("Drop a folder to add it as a project")).not.toBeInTheDocument();
      });

      drop?.onDrop(["D:\\Work\\one", "D:\\Work\\two"]);
      await waitFor(() => {
        expect(callsTo("project_add")).toEqual([
          [{ path: "D:\\Work\\one" }],
          [{ path: "D:\\Work\\two" }],
        ]);
      });
    });
  });

  describe("removing", () => {
    it("asks first, naming the project and saying its files stay", async () => {
      const user = userEvent.setup();
      let list = [shop, blog];
      backendWith({
        projects: () => list,
        more: {
          project_remove: () => {
            list = [blog];
            return null;
          },
        },
      });
      renderList();

      await user.click(
        await within(await rowOf("Shop")).findByRole("button", { name: "Remove Shop" }),
      );

      const dialog = screen.getByRole("dialog");
      expect(dialog).toHaveTextContent("Remove Shop?");
      expect(dialog).toHaveTextContent(
        "The folder D:\\Work\\shop and the files in it are not touched.",
      );
      expect(callsTo("project_remove")).toEqual([]);
      await user.click(within(dialog).getByRole("button", { name: "Remove project" }));

      await waitFor(() => {
        expect(screen.queryByRole("link", { name: "Shop" })).not.toBeInTheDocument();
      });
      expect(callsTo("project_remove")).toEqual([[{ id: "shop" }]]);
      expect(screen.getByText("Removed Shop")).toBeInTheDocument();
      expect(screen.getByRole("link", { name: "Blog" })).toBeInTheDocument();
    });

    it("keeps the project when the dialog is cancelled", async () => {
      const user = userEvent.setup();
      backendWith();
      renderList();

      await user.click(
        await within(await rowOf("Shop")).findByRole("button", { name: "Remove Shop" }),
      );
      await user.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: "Keep project" }),
      );

      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(callsTo("project_remove")).toEqual([]);
    });
  });
});
