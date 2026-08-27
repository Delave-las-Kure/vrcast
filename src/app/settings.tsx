/**
 * T324 — the settings in one place rather than two.
 *
 * **Two stores holding one choice drift apart in silence.** The theme lived in `localStorage`,
 * the language beside it, and all the while the core's database had `theme` and `language`
 * columns nobody read. A person would see one theme while the settings screen showed another,
 * and they cannot fix that, because the second place is not visible to them.
 *
 * The one place is the core. Not because it is better, but because there is one of it for every
 * window, and it survives the interface being reinstalled: the settings sit in the same
 * database as the server profiles and travel with it.
 *
 * **Everything that writes settings writes from here.** `settings_set` takes the whole object,
 * so two independent writers each holding a copy would overwrite one another — a theme saved
 * over a stale snapshot would bring back the old language. Hence one copy, and it lives here.
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

import { ipc } from "../shared/ipc";
import type { AppError, Settings } from "../shared/contract";

interface SettingsContextValue {
  /** `null` until they have been read. Not "the defaults": showing somebody else's choice as
   *  theirs means flashing a light theme at a person who chose the dark one. */
  settings: Settings | null;
  update: (patch: Partial<Settings>) => void;
  /** What would not save. The appearance screen shows it; nothing else needs to know. */
  error: AppError | null;
}

/**
 * A stand-in for when there is no provider.
 *
 * That is how the interface tests live: they raise one screen rather than the whole
 * application. The stand-in is named as one and keeps nothing — no quiet second store can grow
 * out of it.
 */
const NOTHING: SettingsContextValue = {
  settings: null,
  update: () => {},
  error: null,
};

const SettingsContext = createContext<SettingsContextValue>(NOTHING);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .settingsGet()
      .then((got) => {
        if (alive) setSettings(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, []);

  const update = useCallback((patch: Partial<Settings>) => {
    setSettings((current) => {
      if (!current) return current;
      const merged = { ...current, ...patch };
      // Applied at once, saved after. Waiting for the database before repainting puts a lag
      // on the switch, which a person reads as "it did not take" — and they press it again.
      setError(null);
      ipc.settingsSet(merged).catch((e: AppError) => setError(e));
      return merged;
    });
  }, []);

  const value = useMemo(() => ({ settings, update, error }), [settings, update, error]);
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsContextValue {
  return useContext(SettingsContext);
}
