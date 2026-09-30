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
  useRef,
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
  const [settings, setSettingsState] = useState<Settings | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  /**
   * T658 — saves go out one at a time, and what was changed while one was out goes after it.
   *
   * Every change used to send its own whole snapshot at once and take whichever answer came
   * back as the truth. Two quick changes (the theme, then the mascot off) meant two saves in
   * flight; when the first one's answer came back last it put `mascot: true` back on screen,
   * and the next change sent that back to the core — the choice undone twice over (QA-24A
   * №9). The answers cannot simply be ignored either: the core may keep something other than
   * what was sent (T546 clamps `concurrent_heavy_tasks`), and the screen must show what was
   * kept.
   *
   * So: at most one `settings_set` in flight. Changes made meanwhile are collected
   * (`pending`) and shown at once. When the answer comes, it is the base, and the collected
   * changes are laid over it — the core's corrections to everything else stand, the person's
   * newer choices win for what they touched — and that goes out as the next save. Only when
   * nothing is waiting does the answer alone become what is shown.
   *
   * Refs rather than state for the bookkeeping: it must be read and written in the same tick
   * as the change, not on the next render; `shown` mirrors what is on screen for the same
   * reason.
   */
  const shown = useRef<Settings | null>(null);
  const pending = useRef<Partial<Settings> | null>(null);
  const saving = useRef(false);

  const show = useCallback((next: Settings) => {
    shown.current = next;
    setSettingsState(next);
  }, []);

  useEffect(() => {
    let alive = true;
    ipc
      .settingsGet()
      .then((got) => {
        if (alive) show(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, [show]);

  const flush = useCallback(() => {
    if (saving.current || !pending.current || !shown.current) return;
    // Everything on screen — the collected changes are already in it.
    const sending = shown.current;
    pending.current = null;
    saving.current = true;
    ipc
      .settingsSet(sending)
      .then((saved) => {
        saving.current = false;
        // T546: the core may not keep what was sent. Its answer is the base; what was
        // changed since it was asked goes over it and is saved next.
        show(pending.current ? { ...saved, ...pending.current } : saved);
        flush();
      })
      .catch((e: AppError) => {
        saving.current = false;
        setError(e);
        // What is on screen stays as chosen; whatever was changed after this save is
        // still worth saving.
        flush();
      });
  }, [show]);

  const update = useCallback(
    (patch: Partial<Settings>) => {
      const current = shown.current;
      if (!current) return;
      // Applied at once, saved after. Waiting for the database before repainting puts a lag
      // on the switch, which a person reads as "it did not take" — and they press it again.
      setError(null);
      show({ ...current, ...patch });
      pending.current = { ...pending.current, ...patch };
      flush();
    },
    [show, flush],
  );

  const value = useMemo(() => ({ settings, update, error }), [settings, update, error]);
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsContextValue {
  return useContext(SettingsContext);
}
