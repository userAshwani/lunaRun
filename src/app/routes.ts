import {
  Boxes,
  FolderKanban,
  LayoutDashboard,
  Server,
  Settings,
  Stethoscope,
  type LucideIcon,
} from "lucide-react";

/** Sidebar pages, in display order. The single place where route paths are defined (D-011). */
export const NAV_ITEMS = [
  { id: "dashboard", path: "/", icon: LayoutDashboard, group: "workspace" },
  { id: "projects", path: "/projects", icon: FolderKanban, group: "workspace" },
  { id: "runtimes", path: "/runtimes", icon: Boxes, group: "environment" },
  { id: "services", path: "/services", icon: Server, group: "environment" },
  { id: "settings", path: "/settings", icon: Settings, group: "system" },
  { id: "diagnostics", path: "/diagnostics", icon: Stethoscope, group: "system" },
] as const satisfies readonly {
  id: string;
  path: string;
  icon: LucideIcon;
  group: NavGroupId;
}[];

export type NavGroupId = "workspace" | "environment" | "system";

/** Sidebar section order. */
export const NAV_GROUPS: readonly NavGroupId[] = ["workspace", "environment", "system"];

export type PageId = (typeof NAV_ITEMS)[number]["id"];
