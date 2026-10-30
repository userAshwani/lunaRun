import { Fragment } from "react";
import { useTranslation } from "react-i18next";
import { NavLink, Outlet } from "react-router";
import appIcon from "@/assets/app-icon.png";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { cn } from "@/lib/utils";
import { NAV_GROUPS, NAV_ITEMS } from "./routes";

export function Layout() {
  const { t } = useTranslation();
  return (
    <div className="flex h-screen bg-background">
      <aside className="flex w-60 shrink-0 flex-col border-r bg-card">
        <div className="flex items-center gap-3 px-5 py-5">
          {/* Decorative: the name next to it already says "lunaRun". 128 px source, shown at 36 px. */}
          <img src={appIcon} alt="" width={36} height={36} className="size-9 shrink-0" />
          <div className="min-w-0 leading-tight">
            <p className="text-base font-semibold">{t("app.name")}</p>
            <p className="truncate text-xs text-muted-foreground">{t("app.tagline")}</p>
          </div>
        </div>

        <nav aria-label={t("nav.label")} className="flex-1 overflow-y-auto px-3 pb-4">
          {NAV_GROUPS.map((group) => (
            <Fragment key={group}>
              <p
                id={`nav-group-${group}`}
                className="mt-5 mb-1.5 px-2 text-xs font-medium tracking-wide text-muted-foreground uppercase first:mt-1"
              >
                {t(`nav.groups.${group}`)}
              </p>
              <ul aria-labelledby={`nav-group-${group}`} className="flex flex-col gap-0.5">
                {NAV_ITEMS.filter((item) => item.group === group).map(
                  ({ id, path, icon: Icon }) => (
                    <li key={id}>
                      <NavLink
                        to={path}
                        end={path === "/"}
                        className={({ isActive }) =>
                          cn(
                            "flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm font-medium text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            isActive &&
                              "bg-primary-soft text-primary-soft-foreground hover:bg-primary-soft hover:text-primary-soft-foreground",
                          )
                        }
                      >
                        <Icon className="size-[18px]" aria-hidden />
                        {t(`nav.${id}`)}
                      </NavLink>
                    </li>
                  ),
                )}
              </ul>
            </Fragment>
          ))}
        </nav>

        <div className="flex flex-col gap-3 border-t p-4">
          <ThemeSwitcher />
          <p className="px-1 text-xs text-muted-foreground tabular-nums">
            {t("app.version", { version: __APP_VERSION__ })}
          </p>
        </div>
      </aside>

      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-6xl px-8 py-8">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
