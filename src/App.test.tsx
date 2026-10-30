import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RouterProvider, createMemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { routes } from "@/app/router";
import { NAV_ITEMS } from "@/app/routes";
import { ThemeProvider } from "@/app/theme-provider";
import { createQueryClient } from "@/lib/query-client";

function renderAt(path: string) {
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ThemeProvider>
        <RouterProvider router={router} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove("dark");
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("app shell", () => {
  it("groups the sidebar and shows the app version", () => {
    renderAt("/");
    const nav = screen.getByRole("navigation", { name: "Main navigation" });
    for (const group of ["Workspace", "Environment", "System"]) {
      expect(within(nav).getByText(group)).toBeInTheDocument();
    }
    expect(screen.getByText(/^Version \d+\.\d+\.\d+/)).toBeInTheDocument();
  });

  it("shows the lunaRun icon next to the name as a decorative image", () => {
    renderAt("/");
    const icon = document.querySelector("aside img");
    expect(icon).toHaveAttribute("src", expect.stringContaining("app-icon"));
    expect(icon).toHaveAttribute("alt", ""); // decorative: the visible name already says "lunaRun"
    expect(screen.getAllByText("lunaRun").length).toBeGreaterThan(0);
  });

  it("shows every sidebar page and marks the current one", () => {
    renderAt("/runtimes");
    const nav = screen.getByRole("navigation", { name: "Main navigation" });
    expect(within(nav).getAllByRole("link")).toHaveLength(NAV_ITEMS.length);
    expect(within(nav).getByRole("link", { name: "Runtimes" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(within(nav).getByRole("link", { name: "Dashboard" })).not.toHaveAttribute(
      "aria-current",
    );
  });

  it("navigates between pages", async () => {
    renderAt("/");
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Services" }));
    expect(screen.getByRole("heading", { name: "Services" })).toBeInTheDocument();
  });

  it.each(NAV_ITEMS)("renders the $id page at $path", ({ path, id }) => {
    renderAt(path);
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      id.charAt(0).toUpperCase() + id.slice(1),
    );
  });
});

describe("theme", () => {
  it("applies and persists the dark theme", async () => {
    renderAt("/");
    await userEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement).toHaveClass("dark");
    expect(localStorage.getItem("lunarun.theme")).toBe("dark");
    expect(screen.getByRole("radio", { name: "Dark" })).toBeChecked();

    await userEvent.click(screen.getByRole("radio", { name: "Light" }));
    expect(document.documentElement).not.toHaveClass("dark");
  });

  it("restores a stored choice on start", () => {
    localStorage.setItem("lunarun.theme", "dark");
    renderAt("/");
    expect(document.documentElement).toHaveClass("dark");
  });

  it("defaults to light, never dark", () => {
    renderAt("/");
    expect(screen.getByRole("radio", { name: "Light" })).toBeChecked();
    expect(document.documentElement).not.toHaveClass("dark");
  });

  it("ignores an invalid stored value", () => {
    localStorage.setItem("lunarun.theme", "purple");
    renderAt("/");
    expect(screen.getByRole("radio", { name: "Light" })).toBeChecked();
  });

  it("still works when storage is blocked", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    renderAt("/");
    await userEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement).toHaveClass("dark");
  });
});
