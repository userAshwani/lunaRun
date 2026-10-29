import { createContext, useContext } from "react";

export type Theme = "light" | "dark" | "system";

export interface ThemeContextValue {
  /** The user's choice. */
  theme: Theme;
  /** What is actually applied right now (system resolved to light or dark). */
  resolved: "light" | "dark";
  setTheme: (theme: Theme) => void;
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error("useTheme must be used inside <ThemeProvider>");
  return value;
}
