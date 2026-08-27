/**
 * T322, T327, T328 — the mascot, its moods and its switch (FR-102, FR-103).
 *
 * **A mascot that is turned off is not loaded at all.** Otherwise "off" means only "not
 * visible" while it spends just as much — on exactly the weak machine where somebody turned it
 * off. The drawing is a file of its own, fetched on demand: with the setting off the `lazy`
 * branch never runs, and nothing is asked for. That is checked by the **absence of a request**
 * rather than the absence of a picture (T330): there is no picture either when one simply did
 * not render.
 *
 * **The mood comes from the same events as the task screen.** The rules live in `state.ts` and
 * only the subscription lives here: kept apart so the rules can be checked without a screen.
 */

import { Suspense, lazy, useEffect, useMemo, useRef, useState } from "react";

import {
  emptyMind,
  moodLabel,
  moodOf,
  onDone,
  onProgress,
  onViewers,
  type Mind,
  type Mood,
} from "./state";
import { useSettings } from "../../app/settings";
import { useT } from "../../shared/i18n";
import { onTaskDone, onTaskProgress, onViewersUpdate } from "../../shared/ipc";

/**
 * Fetched on demand, and that is the whole point of it here.
 *
 * Lifted to module level, because a `lazy` inside the component body would make a new lazy
 * module on every render — and the drawing would be asked for again and again.
 */
const Drawing = lazy(() => import("./MascotDrawing"));

/** How often to work the mood out again: a moment from an event expires on its own. */
const TICK_MS = 500;

export function Mascot() {
  const t = useT();
  const { settings, error } = useSettings();
  const mind = useRef<Mind>(emptyMind());
  const [mood, setMood] = useState<Mood>("idle");

  /**
   * Until it is known: not shown, and — the point — **not loaded**.
   *
   * The temptation was the other way round. The mascot is on by default, so "assume on until
   * we have read the setting" looked reasonable. And it worked: the drawing went off to be
   * fetched on the very first render, before the core could answer "off", and the setting
   * stopped it from nothing whatever. Caught by the `mascot-off` check, which exists for this.
   *
   * If the core did not answer at all (`error`), the default is taken: hiding the mascot
   * because a database is out of reach would punish a person for somebody else's fault.
   */
  const known = settings !== null || error !== null;
  const shown = known && settings?.mascot !== false;

  useEffect(() => {
    if (!shown) return;
    let alive = true;
    const refresh = () => {
      if (alive) setMood(moodOf(mind.current, Date.now()));
    };

    const stops: Promise<() => void>[] = [
      onTaskProgress((e) => {
        mind.current = onProgress(mind.current, e);
        refresh();
      }),
      onTaskDone((e) => {
        mind.current = onDone(mind.current, e, Date.now());
        refresh();
      }),
      onViewersUpdate((e) => {
        mind.current = onViewers(mind.current, e);
        refresh();
      }),
    ];

    // A moment from an event expires with no event of its own, and without this the mascot
    // would be stuck on "it worked" until the next task — which is to say, sometimes for
    // good.
    const timer = setInterval(refresh, TICK_MS);

    return () => {
      alive = false;
      clearInterval(timer);
      for (const stop of stops) void stop.then((f) => f());
    };
  }, [shown]);

  const words = useMemo(() => t.ui.appearance as unknown as Record<string, string>, [t]);

  if (!shown) return null;

  return (
    <div className="mascot-slot" data-testid="mascot-slot">
      {/* Empty while the drawing is on its way. No placeholder, deliberately: flashing a grey
          circle and then swapping the mascot in is two movements where one will do. */}
      <Suspense fallback={null}>
        <Drawing mood={mood} label={moodLabel(mood, words)} />
      </Suspense>
    </div>
  );
}
