import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTheme, type Theme } from "@/app/theme-context";
import { cn } from "@/lib/utils";

const OPTIONS: readonly { id: Theme; icon: LucideIcon }[] = [
  { id: "light", icon: Sun },
  { id: "dark", icon: Moon },
  { id: "system", icon: Monitor },
];

/** Segmented control for Light / Dark / System. */
export function ThemeSwitcher() {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();
  return (
    <div
      role="radiogroup"
      aria-label={t("theme.label")}
      className="grid grid-cols-3 gap-1 rounded-lg bg-muted p-1"
    >
      {OPTIONS.map(({ id, icon: Icon }) => (
        <button
          key={id}
          type="button"
          role="radio"
          aria-checked={theme === id}
          aria-label={t(`theme.${id}`)}
          title={t(`theme.${id}`)}
          onClick={() => {
            setTheme(id);
          }}
          className={cn(
            "flex items-center justify-center rounded-md py-1.5 text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
            theme === id && "bg-card text-foreground shadow-xs",
          )}
        >
          <Icon className="size-4" aria-hidden />
        </button>
      ))}
    </div>
  );
}
