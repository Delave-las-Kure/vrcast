/**
 * T395, T400 — what the close button will do, said before it is pressed.
 *
 * **Why this is worth a paragraph on a settings screen.** The button does two different
 * things depending on the desktop, and the difference matters: on one it hides a window while
 * hours of encoding carry on, on the other it ends them. Somebody who expects the first and
 * gets the second loses an afternoon's work; somebody who expects the second and gets the
 * first has an application running they believe they closed.
 *
 * The core decides which — only it can ask whether an AppIndicator library is there — and
 * this says the answer out loud rather than leaving it to be discovered.
 */

import { useEffect, useState } from "react";

import type { TrayState } from "../../shared/contract";
import { ipc } from "../../shared/ipc";
import { useT } from "../../shared/i18n";

export function CloseButton() {
  const t = useT();
  const words = t.ui.appearance;
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

  return (
    <fieldset>
      <legend>{words.closeTitle}</legend>
      <p className="appearance__means" data-testid="close-behaviour">
        {tray === "installed"
          ? words.closeHides
          : tray === "unavailable"
            ? words.closeExits
            : words.closeUnknown}
      </p>
    </fieldset>
  );
}
