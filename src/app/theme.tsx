/**
 * Appearance: dark, light, and following the system (FR-100).
 *
 * The choice is stored locally and survives a restart. The default is "as in the
 * system": that is what a person expects without opening the settings at all.
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

export type ThemeChoice = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "vrcast.theme";

interface ThemeContextValue {
  choice: ThemeChoice;
  resolved: ResolvedTheme;
  setChoice: (c: ThemeChoice) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function readStored(): ThemeChoice {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    // Storage can be unavailable. Not a reason to refuse to start.
  }
  return "system";
}

function systemTheme(): ResolvedTheme {
  if (typeof window === "undefined" || !window.matchMedia) return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [choice, setChoiceState] = useState<ThemeChoice>(readStored);
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

  const resolved: ResolvedTheme = choice === "system" ? system : choice;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setChoice = useCallback((c: ThemeChoice) => {
    setChoiceState(c);
    try {
      localStorage.setItem(STORAGE_KEY, c);
    } catch {
      // Not saved — survivable: the choice still applies for this run.
    }
  }, []);

  const value = useMemo(() => ({ choice, resolved, setChoice }), [choice, resolved, setChoice]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme used outside ThemeProvider");
  return ctx;
}
