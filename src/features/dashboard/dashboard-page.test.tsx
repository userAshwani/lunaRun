import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { JobInfo } from "@/bindings/JobInfo";
import { ToastProvider } from "@/components/toast";
import { SERVICE_STATE_EVENT } from "@/lib/events";
import { createQueryClient } from "@/lib/query-client";
import { backendError, callsTo, emitEvent, resetBackend, respondTo } from "@/test/backend";
import { blog, project, shop, siteEntry, sitesView } from "@/test/projects-fixtures";
import { installedPhp } from "@/test/runtimes-fixtures";
import { caddyRow, mysqlRow, phpPool, status } from "@/test/services-fixtures";
import { DashboardPage } from "./dashboard-page";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

function renderDashboard() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <MemoryRouter initialEntries={["/"]}>
        <ToastProvider>
          <Routes>
            <Route path="/" element={<DashboardPage />} />
            <Route path="/projects/:id" element={<p>project page</p>} />
            <Route path="*" element={<p>another page</p>} />
          </Routes>
        </ToastProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const runningCaddy = {
  ...caddyRow,
  state: "running" as const,
  ports: [80],
  running: 1,
  members: [status("caddy", { state: "running", pid: 7 })],
};
const runningPool = {
  ...phpPool,
  state: "running" as const,
  running: 4,
  members: phpPool.members.map((m) => ({ ...m, state: "running" as const })),
};

function failedJob(id: string, key: string, label: string, message: string): JobInfo {
  return {
    id,
    key,
    label,
    state: "failed",
    phase: "",
    bytesDone: 0,
    bytesTotal: null,
    message: null,
    error: { code: "NETWORK", message },
  };
}

function backend(handlers: Record<string, (args?: unknown) => unknown> = {}) {
  respondTo({
    services_list: () => [runningCaddy, runningPool, mysqlRow],
    projects_list: () => [shop, blog],
    sites_list: () =>
      sitesView(
        [
          siteEntry("shop"),
          siteEntry("blog", {
            port: 8102,
            address: "http://127.0.0.1:8102",
            served: true,
            running: true,
          }),
        ],
        "running",
      ),
    runtimes_installed: () => [installedPhp],
    jobs_list: () => [],
    project_start: () => ({ jobId: "j1", key: "project:shop" }),
    project_open: () => null,
    ...handlers,
  });
}

beforeEach(() => {
  resetBackend();
});

describe("the dashboard", () => {
  it("shows the shape of each section while it loads, not numbers that are not true yet", () => {
    // Nothing answers: every query stays pending.
    const never = () => new Promise(() => undefined);
    respondTo({
      services_list: never,
      projects_list: never,
      sites_list: never,
      runtimes_installed: never,
      jobs_list: never,
    });
    renderDashboard();
    expect(screen.getByRole("status", { name: "Loading projects" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Loading services" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Checking for problems" })).toBeInTheDocument();
    expect(screen.queryByText("Get started")).not.toBeInTheDocument();
    expect(screen.queryByText("No problems")).not.toBeInTheDocument();
  });

  it("on a fresh install: honest zeros, the three steps, and an empty state that says what to do", async () => {
    backend({
      services_list: () => [],
      projects_list: () => [],
      sites_list: () => sitesView([], "notInstalled"),
      runtimes_installed: () => [],
    });
    renderDashboard();
    expect(await screen.findByText("Get started")).toBeInTheDocument();
    expect(screen.getByText("Nothing installed yet")).toBeInTheDocument();
    expect(screen.getByText("Nothing to run yet")).toBeInTheDocument();
    expect(screen.getByText("No projects added yet")).toBeInTheDocument();
    expect(screen.queryByText("Done")).not.toBeInTheDocument();
    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Add a project" })).toHaveAttribute(
      "href",
      "/projects",
    );
    expect(screen.getByText("Nothing is running")).toBeInTheDocument();
    expect(screen.getByText("No problems")).toBeInTheDocument();
  });

  it("ticks the steps that are done, and goes away once something is installed and a project added", async () => {
    backend({ projects_list: () => [], sites_list: () => sitesView([]) });
    const view = renderDashboard();
    expect(await screen.findByText("Get started")).toBeInTheDocument();
    expect(screen.getAllByText("Done")).toHaveLength(1);
    view.unmount();

    resetBackend();
    backend();
    renderDashboard();
    expect(await screen.findByRole("link", { name: "Blog" })).toBeInTheDocument();
    expect(screen.queryByText("Get started")).not.toBeInTheDocument();
  });

  it("counts what runs, lists the running services with their ports, and names what is installed", async () => {
    backend();
    renderDashboard();
    const services = await screen.findByText("of 3 services");
    expect(services.previousElementSibling).toHaveTextContent("2");
    expect(screen.getByText("1 running")).toBeInTheDocument();
    expect(screen.getByText("PHP")).toBeInTheDocument();
    expect(await screen.findByText("Caddy")).toBeInTheDocument();
    expect(screen.getByText("Port 80")).toBeInTheDocument();
    expect(screen.getByText("PHP 8.3.36")).toBeInTheDocument();
    expect(screen.getByText("Port 9100–9103")).toBeInTheDocument();
    // A stopped database is not "running".
    expect(screen.queryByText("MySQL 8.4")).not.toBeInTheDocument();
  });

  it("lists the running project first, with Stop and Open in browser; a stopped one has Start", async () => {
    const user = userEvent.setup();
    backend();
    renderDashboard();
    const links = await screen.findAllByRole("link", { name: /^(Shop|Blog)$/ });
    expect(links.map((link) => link.textContent)).toEqual(["Blog", "Shop"]);
    expect(screen.getByText("http://127.0.0.1:8102")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop Blog" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Open Shop in the browser" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Open Blog in the browser" }));
    expect(callsTo("project_open")).toEqual([[{ id: "blog", target: "browser" }]]);
    await user.click(screen.getByRole("button", { name: "Start Shop" }));
    expect(callsTo("project_start")).toEqual([[{ id: "shop" }]]);
  });

  it("says why a project cannot start, and offers its page", async () => {
    backend({
      sites_list: () => sitesView([siteEntry("shop", { state: "noPhp", address: null })]),
      projects_list: () => [shop],
    });
    renderDashboard();
    const start = await screen.findByRole("button", { name: "Start Shop" });
    await waitFor(() => {
      expect(start).toBeDisabled();
    });
    expect(start).toHaveAttribute("title", expect.stringMatching(/PHP/));
  });

  it("shows a failed start on the project's row, with a link to the details", async () => {
    backend({
      jobs_list: () => [
        failedJob("j1", "project:shop", "Start Shop", "port 8101 is taken by httpd.exe"),
      ],
    });
    renderDashboard();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Could not start/);
    expect(within(alert).getByRole("link", { name: "Details" })).toHaveAttribute(
      "href",
      "/projects/shop",
    );
    // It is told once: not in Problems too.
    expect(await screen.findByText("No problems")).toBeInTheDocument();
  });

  it("lists more than five projects as a link to all of them", async () => {
    const many = Array.from({ length: 6 }, (_, n) => project(`p${String(n)}`));
    backend({ projects_list: () => many, sites_list: () => sitesView([]) });
    renderDashboard();
    expect(await screen.findByRole("link", { name: "View all 6 projects" })).toHaveAttribute(
      "href",
      "/projects",
    );
    expect(screen.getAllByRole("button", { name: /^Start / })).toHaveLength(5);
  });

  it("lists what is wrong now, each with the page where it is fixed", async () => {
    const error = { code: "SERVICE_START_FAILED" as const, message: "mysqld exited with code 1" };
    backend({
      services_list: () => [
        runningCaddy,
        {
          ...mysqlRow,
          state: "crashed",
          members: [status("mysql@8.4", { state: "crashed", gaveUp: true, error })],
        },
      ],
      jobs_list: () => [
        failedJob("j2", "node@24.1.0", "Install Node.js 24.1.0", "the download stopped at 40%"),
      ],
      sites_list: () =>
        sitesView([siteEntry("shop"), siteEntry("blog")], "running", {
          code: "RELOAD_FAILED",
          message: "Caddyfile:4: unknown directive",
        }),
    });
    renderDashboard();
    expect(
      await screen.findByText("MySQL 8.4 crashed and was not restarted again"),
    ).toBeInTheDocument();
    expect(screen.getByText("mysqld exited with code 1")).toBeInTheDocument();
    expect(screen.getByText("Install Node.js 24.1.0 failed")).toBeInTheDocument();
    expect(screen.getByText("the download stopped at 40%")).toBeInTheDocument();
    expect(
      screen.getByText("The web server did not accept the last change to the sites"),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open" })).toHaveAttribute("href", "/runtimes");
    expect(screen.getByRole("link", { name: "Open Diagnostics" })).toHaveAttribute(
      "href",
      "/diagnostics",
    );
    expect(screen.queryByText("No problems")).not.toBeInTheDocument();
  });

  it("says what could not be read, does not claim there are no problems, and tries again", async () => {
    const user = userEvent.setup();
    let fail = true;
    backend({
      projects_list: () =>
        fail ? backendError("STATE_CORRUPT", "config/projects.json is not valid JSON") : [shop],
      services_list: () =>
        fail ? backendError("INTERNAL", "the supervisor is not ready") : [runningCaddy],
    });
    renderDashboard();
    expect(await screen.findByText("The projects could not be read")).toBeInTheDocument();
    expect(screen.getByText("config/projects.json is not valid JSON")).toBeInTheDocument();
    expect(screen.getByText("The services could not be read")).toBeInTheDocument();
    expect(screen.getByText("Could not check for problems")).toBeInTheDocument();
    expect(screen.getAllByText("Could not be read")).toHaveLength(2);
    expect(screen.queryByText("No problems")).not.toBeInTheDocument();
    expect(screen.queryByText("Get started")).not.toBeInTheDocument();

    fail = false;
    for (const button of screen.getAllByRole("button", { name: /try again/i })) {
      await user.click(button);
    }
    expect(await screen.findByRole("link", { name: "Shop" })).toBeInTheDocument();
    expect(await screen.findByText("No problems")).toBeInTheDocument();
  });

  it("follows the services as they change, without polling", async () => {
    let rows = [caddyRow];
    backend({ services_list: () => rows });
    renderDashboard();
    expect(await screen.findByText("Nothing is running")).toBeInTheDocument();
    const asked = callsTo("services_list").length;

    rows = [runningCaddy];
    emitEvent(SERVICE_STATE_EVENT, status("caddy", { state: "running", pid: 7 }));
    expect(await screen.findByText("Port 80")).toBeInTheDocument();
    expect(callsTo("services_list").length).toBeGreaterThan(asked);
  });
});
