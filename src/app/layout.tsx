import { Moon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { NavLink, Outlet } from "react-router";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { cn } from "@/lib/utils";
import { NAV_ITEMS } from "./routes";

export function Layout() {
  const { t } = useTranslation();
  return (
    <div className="flex h-screen">
      <aside className="flex w-56 shrink-0 flex-col border-r bg-card p-3">
        <div className="flex items-center gap-2 px-2 py-3">
          <Moon className="size-5 text-primary" aria-hidden />
          <span className="text-lg font-semibold">{t("app.name")}</span>
        </div>
        <nav aria-label={t("nav.label")} className="mt-2 flex flex-1 flex-col gap-1">
          {NAV_ITEMS.map(({ id, path, icon: Icon }) => (
            <NavLink
              key={id}
              to={path}
              end={path === "/"}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-2 rounded-md px-2 py-2 text-sm font-medium text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
                  isActive && "bg-accent text-accent-foreground",
                )
              }
            >
              <Icon className="size-4" aria-hidden />
              {t(`nav.${id}`)}
            </NavLink>
          ))}
        </nav>
        <ThemeSwitcher />
      </aside>
      <main className="flex-1 overflow-auto p-8">
        <Outlet />
      </main>
    </div>
  );
}
