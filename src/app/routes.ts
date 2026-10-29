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
  { id: "dashboard", path: "/", icon: LayoutDashboard },
  { id: "projects", path: "/projects", icon: FolderKanban },
  { id: "runtimes", path: "/runtimes", icon: Boxes },
  { id: "services", path: "/services", icon: Server },
  { id: "settings", path: "/settings", icon: Settings },
  { id: "diagnostics", path: "/diagnostics", icon: Stethoscope },
] as const satisfies readonly { id: string; path: string; icon: LucideIcon }[];

export type PageId = (typeof NAV_ITEMS)[number]["id"];
