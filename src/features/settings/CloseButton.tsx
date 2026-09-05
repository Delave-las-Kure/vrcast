/**
 * T395, T400, T399 — what the close button will do, said before it is pressed, and chosen.
 *
 * **Why this is worth a section on a settings screen.** The button does two different things
 * depending on the desktop, and the difference matters: on one it hides a window while hours
 * of encoding carry on, on the other it ends them. Somebody who expects the first and gets
 * the second loses an afternoon's work; somebody who expects the second and gets the first
 * has an application running they believe they closed.
 *
 * The core decides what is *possible* — only it can ask whether an AppIndicator library is
 * there — and the person decides what happens where both are possible. Where there is no
 * tray no choice is offered, because there is none to make: a window hidden into nothing is
 * the worst outcome available, so the button closes and the setting cannot ask otherwise.
 *
 * ⚠ **Rendered by nobody until 2026-09-05.** This section existed, was worded in both
 * languages and had three tests, and no screen mounted it — the tests reached it by importing
 * it directly. A component nothing mounts is indistinguishable from one nobody wrote, which
 * is the third time this project has met that shape (T366 for commands, T443 for a screen).
 * `src/__tests__/reachable.test.ts` is the guard that came out of it.
 */

import { useEffect, useState } from "react";

import type { TrayState } from "../../shared/contract";
import { ipc } from "../../shared/ipc";
import { useT } from "../../shared/i18n";
import { useSettings } from "../../app/settings";

export function CloseButton() {
  const t = useT();
  const words = t.ui.appearance;
  const { settings, update } = useSettings();
  const [tray, setTray] = useState<TrayState | null>(null);
  const [asked, setAsked] = useState(false);

  useEffect(() => {
    let alive = true;
    ipc
      .trayState()
      .then((state) => {
        if (alive) setTray(state);
      })
      // Not knowing is its own answer and is said as one below, rather than passed off as
      // either of the two real ones.
      .catch(() => undefined)
      .finally(() => {
        if (alive) setAsked(true);
      });
    return () => {
      alive = false;
    };
  }, []);

  if (!asked) return null;

  // The default matches the core's: keep the window where there is somewhere to keep it.
  const toTray = settings?.close_to_tray ?? true;

  return (
    <fieldset>
      <legend>{words.closeTitle}</legend>

      {tray === "installed" && (
        <label>
          <input
            type="checkbox"
            checked={toTray}
            onChange={(e) => update({ close_to_tray: e.target.checked })}
            data-testid="close-to-tray-switch"
          />
          {words.closeToTray}
        </label>
      )}

      {/* What the button will do, in the same words whether it was chosen or decided for
          them. Read from the setting where there is a choice, from the desktop where
          there is not. */}
      <p className="appearance__means" data-testid="close-behaviour">
        {tray === "installed"
          ? toTray
            ? words.closeHides
            : words.closeExits
          : tray === "unavailable"
            ? words.closeExits
            : words.closeUnknown}
      </p>
    </fieldset>
  );
}
