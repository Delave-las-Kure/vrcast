/**
 * T400 — leaving through the tray menu, with the cost named first (FR-086).
 *
 * ⚠ **Until this existed, "Exit" in the tray menu called `app.exit(0)` and that was all.**
 * Somebody with a thirty-gigabyte upload running chose it and lost the transfer without a
 * word. The close button had been taught to warn (T394, T395); the menu item beside it had
 * not, and the two are the only two ways out.
 *
 * **Why a question here rather than a panel.** FR-086 says in as many words that a general
 * "tasks are running, close?" is not enough — it does not let anybody decide. So the
 * consequences arrive per task, by name, from `tasks_on_close`, and the answer is a choice
 * between two named outcomes rather than an acknowledgement.
 *
 * **The core asks; it does not tell.** It counted what is at stake and found something, which
 * is why this is being shown at all — and it exits nothing until `appExit` is called. Where
 * nothing is at stake it never asks: a dialog that always appears and always has one right
 * answer is one people learn to dismiss unread, and then the one that mattered goes with it.
 */

import { useEffect, useState } from "react";
import type { TaskOnClose } from "../../shared/contract";
import { ipc, onAppQuitRequested } from "../../shared/ipc";
import { useT } from "../../shared/i18n";
import { CloseConsequences } from "./CloseConsequences";

/** What is being asked about: the tasks, or the fact that they could not be read. */
type Asking = { items: TaskOnClose[] } | { unknown: true };

export function LeaveConfirm() {
  const t = useT();
  const [asking, setAsking] = useState<Asking | null>(null);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let dropped = false;

    onAppQuitRequested(() => {
      // Asked for on the question, never on mount. Kept fresh in the background it would be
      // a list of tasks read at some earlier moment and shown as the state of things now —
      // and the moment it is shown is the only one a person acts on.
      ipc
        .tasksOnClose()
        .then((items) => setAsking({ items }))
        // **Not knowing is said out loud rather than smoothed over.** An empty list here
        // would read as "nothing to lose", which is the one wrong thing to say: the core
        // asked precisely because it found something.
        .catch(() => setAsking({ unknown: true }));
    })
      .then((unlisten) => {
        if (dropped) unlisten();
        else stop = unlisten;
      })
      .catch(() => {
        // Outside the shell (in tests) there is nothing to listen to.
      });

    return () => {
      dropped = true;
      stop?.();
    };
  }, []);

  if (asking === null) return null;

  return (
    <div className="leave" role="dialog" aria-modal="true" aria-label={t.ui.tasks.leaveQuestion}>
      <div className="dialog" data-testid="leave-confirm">
        <h3>{t.ui.tasks.leaveQuestion}</h3>

        {"unknown" in asking ? (
          <p className="dialog__warning" role="status" data-testid="leave-unknown">
            {t.ui.tasks.leaveUnknown}
          </p>
        ) : (
          <CloseConsequences items={asking.items} />
        )}

        <div className="form__actions">
          {/* Staying is the default focus: the destructive answer should be the one a person
              reaches for on purpose, not the one a stray Enter lands on. */}
          <button type="button" autoFocus onClick={() => setAsking(null)} data-testid="leave-no">
            {t.ui.tasks.leaveCancel}
          </button>
          <button
            type="button"
            className="danger"
            onClick={() => void ipc.appExit()}
            data-testid="leave-yes"
          >
            {t.ui.tasks.leaveConfirm}
          </button>
        </div>
      </div>
    </div>
  );
}
