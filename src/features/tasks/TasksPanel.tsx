/**
 * T100 — the single task screen (FR-082).
 *
 * Every task in one place: uploads, preparation, and whatever comes later. A separate
 * list per kind would mean walking round the whole application to find out what it is
 * busy with.
 *
 * Progress arrives as events rather than by polling — otherwise showing a task that
 * runs for hours would itself become the cause of the stuttering we avoid
 * (SC-009, R-15).
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import type { AppError, Task, TaskKind } from "../../shared/contract";
import type { TaskOnClose } from "../../shared/contract";
import { ipc, onTaskDone, onTaskProgress, toAppError } from "../../shared/ipc";
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { fill, renderError, renderStage } from "../../shared/i18n/render";
import { ErrorNotice } from "../shared/ErrorNotice";
import { CloseConsequences } from "./CloseConsequences";
import { QueueOrder } from "./QueueOrder";

/**
 * Speed and time left, in the language in use.
 *
 * Both take the catalogue rather than composing a sentence, because both need a unit
 * and a separator that differ between languages — and a number formatted one way here
 * and another way on the library screen is what stops people trusting either.
 */
function formatSpeed(bps: number | null, t: Catalogue, lang: Lang): string | null {
  if (bps === null || bps <= 0) return null;
  const mbit = (bps * 8) / 1_000_000;
  const shown = lang === "ru" ? mbit.toFixed(1).replace(".", ",") : mbit.toFixed(1);
  return fill(t.ui.tasks.speed, { mbit: shown }, t, lang);
}

function formatEta(seconds: number | null, t: Catalogue, lang: Lang): string | null {
  if (seconds === null || seconds <= 0) return null;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return fill(t.ui.tasks.etaHours, { h, m }, t, lang);
  if (m > 0) return fill(t.ui.tasks.etaMinutes, { m }, t, lang);
  return t.ui.tasks.etaSoon;
}

export function TasksPanel() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [onClose, setOnClose] = useState<TaskOnClose[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const t = useT();
  const { lang } = useLang();

  const reload = useCallback(async () => {
    try {
      setTasks(await ipc.tasksList());
      setError(null);
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setLoading(false);
    }
    // The consequences of closing come in a separate request: the core works them
    // out, and repeating that arithmetic here would mean disagreeing with it one day.
    // A failure here does not break the task list: this is an aside, not the list.
    try {
      setOnClose(await ipc.tasksOnClose());
    } catch {
      setOnClose([]);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Progress arrives as a stream; the full list is read again only on completion,
  // when the membership changes rather than a number.
  useEffect(() => {
    // Subscribing is asynchronous, and unmounting can happen before it finishes (in
    // development StrictMode guarantees it). Then there is nobody left to unsubscribe
    // from cleanup — so a subscription arriving after it is cancelled on the spot, or
    // handlers pile up for the rest of the session with every visit to the section.
    let cancelled = false;
    const unlisten: Array<() => void> = [];
    const keep = (fn: () => void) => {
      if (cancelled) fn();
      else unlisten.push(fn);
    };

    void onTaskProgress((e) => {
      setTasks((prev) =>
        prev.map((task) =>
          task.id === e.id
            ? {
                ...task,
                state: e.state,
                progress: e.progress,
                stage: e.stage,
                speed_bps: e.speed_bps,
                eta_s: e.eta_s,
              }
            : task,
        ),
      );
    }).then(keep);

    void onTaskDone(() => void reload()).then(keep);

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [reload]);

  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
      await reload();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  // Waiting tasks in the order they will run, not the order they appear in the list:
  // otherwise the queue numbers would not match what the core actually does.
  const queued = useMemo(
    () =>
      tasks
        .filter((task) => task.state === "queued")
        .sort((a, b) => a.queue_order - b.queue_order),
    [tasks],
  );

  if (loading) return <div className="panel">{t.ui.tasks.reading}</div>;

  return (
    <div className="panel">
      <h1>{t.ui.tasks.heading}</h1>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <CloseConsequences items={onClose} />

      <QueueOrder
        queued={queued}
        busy={busy}
        onReorder={(ids) => void act(async () => void (await ipc.tasksReorder(ids)))}
      />

      {tasks.length === 0 ? (
        <p className="muted">{t.ui.tasks.empty}</p>
      ) : (
        <ul className="task-list">
          {tasks.map((task) => (
            <li key={task.id} className={`task task--${task.state}`}>
              <div className="task__head">
                <span className="task__kind">
                  {t.ui.tasks.kinds[task.kind as TaskKind] ?? task.kind}
                </span>
                <span className="task__state">{t.ui.tasks.states[task.state]}</span>
              </div>

              {(task.state === "running" || task.state === "paused") && (
                <div
                  className="progress"
                  role="progressbar"
                  aria-valuenow={Math.round(task.progress * 100)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <div
                    className="progress__fill"
                    style={{ width: `${task.progress * 100}%` }}
                  />
                </div>
              )}

              <div className="task__meta">
                {task.stage && <span>{renderStage(task.stage, t, lang)}</span>}
                {formatSpeed(task.speed_bps, t, lang) && (
                  <span>{formatSpeed(task.speed_bps, t, lang)}</span>
                )}
                {formatEta(task.eta_s, t, lang) && (
                  <span>{formatEta(task.eta_s, t, lang)}</span>
                )}
              </div>

              {task.error && (
                <p className="task__error">{renderError(task.error, t, lang).message}</p>
              )}

              <div className="task__actions">
                {task.state === "running" && (
                  <button onClick={() => void act(() => ipc.taskPause(task.id))}>
                    {t.ui.tasks.pause}
                  </button>
                )}
                {task.state === "paused" && (
                  <button onClick={() => void act(() => ipc.taskResume(task.id))}>
                    {t.ui.tasks.resume}
                  </button>
                )}
                {(task.state === "running" ||
                  task.state === "paused" ||
                  task.state === "queued") && (
                  <button
                    className="button--danger"
                    onClick={() => void act(() => ipc.taskCancel(task.id))}
                  >
                    {t.ui.tasks.stop}
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
