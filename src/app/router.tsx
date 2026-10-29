import { createHashRouter, type RouteObject } from "react-router";
import { DashboardPage } from "@/features/dashboard/dashboard-page";
import { DiagnosticsPage } from "@/features/diagnostics/diagnostics-page";
import { ProjectsPage } from "@/features/projects/projects-page";
import { RuntimesPage } from "@/features/runtimes/runtimes-page";
import { ServicesPage } from "@/features/services/services-page";
import { SettingsPage } from "@/features/settings/settings-page";
import { Layout } from "./layout";

/** Exported separately so tests can mount them in a memory router. */
export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Layout />,
    children: [
      { index: true, element: <DashboardPage /> },
      { path: "projects", element: <ProjectsPage /> },
      { path: "runtimes", element: <RuntimesPage /> },
      { path: "services", element: <ServicesPage /> },
      { path: "settings", element: <SettingsPage /> },
      { path: "diagnostics", element: <DiagnosticsPage /> },
    ],
  },
];

// Hash routing: survives dev reloads and needs no server-side fallback (D-011).
export const router = createHashRouter(routes);
