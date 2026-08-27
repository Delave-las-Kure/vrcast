/**
 * Appearance: dark, light, and following the system (FR-100).
 *
 * **The choice lives in the core and nowhere else** (T324). It used to live in
 * `localStorage` while the core kept a `theme` field of its own that nobody read — two
 * stores of one choice, and two stores of one choice diverge silently. The person sees one
 * theme, the settings say another, and they cannot mend it because they cannot see the
 * second place.
 *
 * The default is "as in the system": that is what a person expects without opening the
 * settings at all. Until the core has answered, the system's is what is shown — not a
 * built-in light, which would flash white at somebody who chose dark.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { useSettings } from "./settings";

export type ThemeChoice = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

interface ThemeContextValue {
  choice: ThemeChoice;
  resolved: ResolvedTheme;
  setChoice: (c: ThemeChoice) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function systemTheme(): ResolvedTheme {
  if (typeof window === "undefined" || !window.matchMedia) return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** What the core stores. `null` there means "as in the system". */
function asChoice(stored: string | null | undefined): ThemeChoice {
  return stored === "light" || stored === "dark" ? stored : "system";
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const { settings, update } = useSettings();
  const [system, setSystem] = useState<ResolvedTheme>(systemTheme);

  // The system is watched even when the system mode is not the chosen one: a person
  // can switch to it at any moment, and it has to take effect straight away.
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setSystem(mq.matches ? "dark" : "light");
    mq.addEventListener?.("change", onChange);
    return () => mq.removeEventListener?.("change", onChange);
  }, []);

  const choice = asChoice(settings?.theme);
  const resolved: ResolvedTheme = choice === "system" ? system : choice;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setChoice = useCallback(
    (c: ThemeChoice) => update({ theme: c === "system" ? null : c }),
    [update],
  );

  const value = useMemo(() => ({ choice, resolved, setChoice }), [choice, resolved, setChoice]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme used outside ThemeProvider");
  return ctx;
}
