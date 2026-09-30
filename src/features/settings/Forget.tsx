/**
 * T356–T358 — "remove my data", from inside the application (FR-114).
 *
 * **Why this lives here and not only in the uninstaller.** Of the three ways this is handed
 * out, exactly one can ask a question at removal time: the Windows uninstaller has its
 * checkbox, a `.deb` runs its removal script with nobody to ask, and an AppImage is not
 * installed at all — it is a file somebody deleted. The application is the one place all three
 * have.
 *
 * **And it is the only one that can reach the secrets.** They sit in the operating system's
 * own store, not in the data directory: neither the checkbox nor `postrm` touches them. Once
 * the application is gone there is nobody left to clear them.
 *
 * **A list, not a promise.** "Delete my data" without one is read differently by everybody who
 * reads it, and the person deciding is the one who cannot check afterwards.
 */

import { useCallback, useEffect, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { formatBytes } from "../../shared/i18n/format";
import { ipc, onTaskDone, onTaskProgress } from "../../shared/ipc";
import type { AppError, Task, WhatWent, WhatWouldGo } from "../../shared/contract";

/**
 * T643 — whether a task counts as going for "remove everything".
 *
 * The core refuses while a task is alive in its engine: queued, running, or paused with its work
 * still held — which is what `can_resume` says of a paused one (T515). A paused row left over from
 * an earlier run is only a row: nothing of it can write anything, and the core lets the removal
 * through over it, so the button does too.
 */
function isGoing(task: Task): boolean {
  return (
    task.state === "queued" ||
    task.state === "running" ||
    (task.state === "paused" && task.can_resume)
  );
}

export function Forget() {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.forget;

  const [would, setWould] = useState<WhatWouldGo | null>(null);
  const [went, setWent] = useState<WhatWent | null>(null);
  const [agreed, setAgreed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  // T643: the ids of the tasks going right now. The core is what refuses (and says so with
  // `FORGET_TASKS_RUNNING`, a command still connecting included); this only keeps the button
  // from offering what would be refused.
  const [going, setGoing] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    let alive = true;
    ipc
      .forgetPreview()
      .then((got) => {
        if (alive) setWould(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unlisten: Array<() => void> = [];
    const keep = (fn: () => void) => {
      if (cancelled) fn();
      else unlisten.push(fn);
    };
    const reload = () => {
      ipc
        .tasksList()
        .then((tasks) => {
          if (cancelled || !Array.isArray(tasks)) return;
          setGoing(new Set(tasks.filter(isGoing).map((task) => task.id)));
        })
        // A list that would not load leaves the button as it was: the core still refuses.
        .catch(() => undefined);
    };
    reload();
    // Progress comes only from a task alive in the engine; a new one is seen at its first report.
    void onTaskProgress((e) => {
      setGoing((prev) => (prev.has(e.id) ? prev : new Set(prev).add(e.id)));
    }).then(keep);
    void onTaskDone(() => reload()).then(keep);
    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, []);

  const remove = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setWent(await ipc.forgetEverything(true));
    } catch (e) {
      setError(e as AppError);
    } finally {
      setBusy(false);
    }
  }, []);

  if (went) {
    return (
      <section className="forget">
        <h3>{words.title}</h3>
        <p data-testid="forget-done">{words.done}</p>
        {went.secrets_left.length > 0 && (
          // Said out loud: somebody told "everything is gone" while entries remain has been
          // told something untrue, and has no way of checking it.
          <p className="forget-warning" data-testid="forget-left">
            {words.secretsLeft(went.secrets_left.join(", "))}
          </p>
        )}
        {!went.data_dir_removed && (
          <p className="forget-warning" data-testid="forget-dir-left">
            {words.dirLeft}
          </p>
        )}
      </section>
    );
  }

  return (
    <section className="forget">
      <h3>{words.title}</h3>
      <p className="appearance__means">{words.means}</p>
      {error && <ErrorNotice error={error} />}

      {would && (
        <>
          <ul className="forget-list" data-testid="forget-list">
            {would.data_dir && (
              <li>
                {words.dataDir}: <code>{would.data_dir}</code> ({formatBytes(would.bytes, lang)})
              </li>
            )}
            <li data-testid="forget-servers">
              {words.servers}: {would.servers.length > 0 ? would.servers.join(", ") : words.none}
            </li>
            <li>
              {words.secrets}: {would.secrets}
            </li>
          </ul>

          {would.locked_out.length > 0 && (
            // **The one loss that cannot be undone.** A server deployed by this application
            // has password logins turned off, and the only key to it is the one in here.
            <div className="forget-danger" data-testid="forget-locked-out">
              <p>{words.lockedOut(would.locked_out.join(", "))}</p>
              <p>{words.lockedOutAdvice}</p>
            </div>
          )}

          <label className="forget-agree">
            <input
              type="checkbox"
              checked={agreed}
              onChange={(e) => setAgreed(e.target.checked)}
              data-testid="forget-agree"
            />
            {words.agree}
          </label>

          {going.size > 0 && (
            <p className="forget-warning" data-testid="forget-tasks-running">
              {words.tasksRunning}
            </p>
          )}

          <button
            type="button"
            className="danger"
            disabled={!agreed || busy || going.size > 0}
            onClick={() => void remove()}
            data-testid="forget-do"
          >
            {busy ? words.removing : words.remove}
          </button>
        </>
      )}
    </section>
  );
}
