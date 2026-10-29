import { useEffect, useMemo, useState, type ReactNode } from "react";
import { ThemeContext, type Theme } from "./theme-context";

// Per-viewer convenience only. Moves into settings.json once the settings backend exists.
const STORAGE_KEY = "lunarun.theme";

const THEMES: readonly Theme[] = ["light", "dark", "system"];

// Light is the default; dark and system are opt-in (CLAUDE.md UI/UX standard).
const DEFAULT_THEME: Theme = "light";

function readStoredTheme(): Theme {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return THEMES.find((theme) => theme === stored) ?? DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME; // storage can be blocked or unavailable
  }
}

function systemQuery(): MediaQueryList | null {
  return typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(readStoredTheme);
  const [systemDark, setSystemDark] = useState(() => systemQuery()?.matches ?? false);

  useEffect(() => {
    const query = systemQuery();
    if (!query) return;
    const onChange = (event: MediaQueryListEvent) => {
      setSystemDark(event.matches);
    };
    query.addEventListener("change", onChange);
    return () => {
      query.removeEventListener("change", onChange);
    };
  }, []);

  const resolved = theme === "system" ? (systemDark ? "dark" : "light") : theme;

  useEffect(() => {
    document.documentElement.classList.toggle("dark", resolved === "dark");
    document.documentElement.style.colorScheme = resolved;
  }, [resolved]);

  const value = useMemo(
    () => ({
      theme,
      resolved,
      setTheme: (next: Theme) => {
        setThemeState(next);
        try {
          localStorage.setItem(STORAGE_KEY, next);
        } catch {
          // Not persisted; the choice still applies for this session.
        }
      },
    }),
    [theme, resolved],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
