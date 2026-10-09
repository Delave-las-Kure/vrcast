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
import { Link } from "react-router-dom";
import type {
  AppError,
  Task,
  TaskKind,
  TaskState,
  VideoState,
  VideoView,
} from "../../shared/contract";
import type { TaskOnClose } from "../../shared/contract";
import {
  ipc,
  onTaskDone,
  onTaskProgress,
  onVideoRemoved,
  onVideoUpdate,
  toAppError,
} from "../../shared/ipc";
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { fill, renderDetail, renderStage } from "../../shared/i18n/render";
import { ErrorFolded, ErrorNotice } from "../shared/ErrorNotice";
import { CloseConsequences } from "./CloseConsequences";
import { QueueOrder } from "./QueueOrder";

/**
 * Speed and time left, in the language in use.
 *
 * Both take the catalogue rather than composing a sentence, because both need a unit
 * and a separator that differ between languages — and a number formatted one way here
 * and another way on the library screen is what stops people trusting either.
 */
/**
 * A failed task's error: one line, and what to do folded under "Details" (T674).
 *
 * Not `ErrorNotice` itself: that one is a dismissible banner with `role="alert"`, and this sits
 * inside a row of a list — thirty failed rows must not speak thirty times. The line and the
 * fold are the same component, so the two cannot drift about what a code means.
 */
function TaskError({ error }: { error: AppError }) {
  return (
    <div className="task__error">
      <ErrorFolded error={error} lineClassName="task__error-message" />
    </div>
  );
}

function formatSpeed(bps: number | null, t: Catalogue, lang: Lang): string | null {
  if (bps === null || bps <= 0) return null;
  const mbit = (bps * 8) / 1_000_000;
  const shown = lang === "ru" ? mbit.toFixed(1).replace(".", ",") : mbit.toFixed(1);
  return fill(t.ui.tasks.speed, { mbit: shown }, t, lang);
}

function formatEta(seconds: number | null, t: Catalogue, lang: Lang): string | null {
  // `null` is "not known yet" and shows nothing (T659). A known zero is less than a second
  // left — the core no longer sends zero for "unknown", so it is not hidden with it.
  if (seconds === null || seconds < 0) return null;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return fill(t.ui.tasks.etaHours, { h, m }, t, lang);
  if (m > 0) return fill(t.ui.tasks.etaMinutes, { m }, t, lang);
  return t.ui.tasks.etaSoon;
}

/** The states a task does not come back from. */
const FINISHED = new Set(["completed", "failed", "cancelled"]);

/** A film that is busy (T710): it gets a row here. Waiting for «Start», finished, stopped on
 *  a problem — those are «Video»'s to show; nothing is running for them. */
const FILM_IN_WORK = new Set<VideoState>(["working", "paused", "cancelling"]);

/** The stage a film is at, as «Video» names it. */
function filmStage(film: VideoView, t: Catalogue): string {
  const stages = t.ui.video.stages as Record<string, string>;
  return stages[film.stage] ?? t.ui.video.queued;
}

/**
 * One film, one row (T710): its name, the stage it is at and how far — the same words as its
 * card on «Video». Its stage's tasks are not listed besides: a person thinks of «the film», and
 * the screen used to show it as two or three rows of kinds of work, plus every earlier run.
 */
function FilmRow({
  film,
  busy,
  onPause,
  onResume,
}: {
  film: VideoView;
  busy: boolean;
  onPause: () => void;
  onResume: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const p = film.progress;
  const waiting = film.state === "working" && (p === null || p.task_state === "queued");
  const stateWord =
    film.state === "paused"
      ? t.ui.video.paused
      : film.state === "cancelling"
        ? t.ui.video.stopping
        : waiting
          ? t.ui.video.queued
          : null;
  const rung =
    p && p.rung !== null && p.rungs > 0
      ? fill(t.ui.video.rungOf, { k: p.rung, n: p.rungs }, t, lang)
      : null;
  const speed = p ? formatSpeed(p.speed_bps, t, lang) : null;
  const eta = p ? formatEta(p.eta_s, t, lang) : null;

  return (
    <li className={`task task--film task--${film.state}`} data-testid={`film-${film.id}`}>
      <div className="task__head">
        <span className="task__batch">{film.title}</span>
        <span className="task__kind">{filmStage(film, t)}</span>
        {stateWord && <span className="task__state">{stateWord}</span>}
      </div>

      {p && !waiting && film.state !== "cancelling" && (
        <div
          className="progress"
          role="progressbar"
          aria-valuenow={Math.round(p.progress * 100)}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div className="progress__fill" style={{ width: `${p.progress * 100}%` }} />
        </div>
      )}

      {(rung || speed || eta) && (
        <div className="task__meta">
          {rung && <span>{rung}</span>}
          {speed && <span>{speed}</span>}
          {eta && <span>{eta}</span>}
        </div>
      )}

      <div className="task__actions">
        {film.state === "working" && (
          <button disabled={busy} onClick={onPause}>
            {t.ui.video.pause}
          </button>
        )}
        {film.state === "paused" && (
          <button disabled={busy} onClick={onResume}>
            {t.ui.video.resume}
          </button>
        )}
        <Link to="/video">{t.ui.tasks.openFilm}</Link>
      </div>
    </li>
  );
}

/**
 * The state a row takes from a progress event (T652, QA-24A №3).
 *
 * **A paused row is not made running by a progress event.** The core leaves a pause only by
 * "carry on", and carrying on first makes the task `queued` — announced as an event of its
 * own, and read back by the list the button reloads — and only then `running`, once it has a
 * place in its lane. So `running` straight over `paused` can only be a report that set out
 * before the pause and arrived after it: taking it would hide "Carry on" from a task that is
 * really standing still. A finished row is not brought back by any event either.
 */
function nextState(shown: TaskState, reported: TaskState): TaskState {
  if (FINISHED.has(shown)) return shown;
  if (shown === "paused" && reported === "running") return shown;
  return reported;
}

export function TasksPanel() {
  const [tasks, setTasks] = useState<Task[]>([]);
  /** The films on «Video» (T710): a film in work is one row here, not its stage's tasks. */
  const [videos, setVideos] = useState<VideoView[]>([]);
  const [onClose, setOnClose] = useState<TaskOnClose[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [stoppedCount, setStoppedCount] = useState<number | null>(null);
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
    // The films, for their rows. Not having them is not a reason to show no tasks: the rows
    // of their tasks then fall back to being shown one by one, as before.
    try {
      setVideos(await ipc.videoList());
    } catch {
      setVideos([]);
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
                state: nextState(task.state, e.state),
                progress: e.progress,
                // An event that says nothing about the stage says nothing about it. Most
                // do not: `report_transfer` sends how fast and how long left, four times a
                // second, with no stage at all — and copying that null over would blank the
                // line under the bar for the rest of the work (T413).
                stage: e.stage ?? task.stage,
                speed_bps: e.speed_bps,
                eta_s: e.eta_s,
              }
            : task,
        ),
      );
    }).then(keep);

    void onTaskDone(() => void reload()).then(keep);

    // A film's row follows the film itself: its stage, its rung, its pause (T710).
    onVideoUpdate((video) =>
      setVideos((prev) => {
        const at = prev.findIndex((v) => v.id === video.id);
        if (at < 0) return [...prev, video];
        if (prev[at].rev > video.rev) return prev;
        const next = prev.slice();
        next[at] = video;
        return next;
      }),
    )
      .then(keep)
      .catch(() => {
        // Outside the shell (in tests) there is nothing to listen to.
      });
    onVideoRemoved((id) => setVideos((prev) => prev.filter((v) => v.id !== id)))
      .then(keep)
      .catch(() => {});

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

  // The films in work (T710), one row each, and the ids that make a task one of theirs. A
  // film's task is its stage's work, and listing it apart — under the name of its kind,
  // «quality measuring on the material», and again for every run — is what made the screen a
  // history of old names rather than a picture of what is going on.
  const filmIds = useMemo(() => new Set(videos.map((v) => v.id)), [videos]);
  const films = videos.filter((v) => FILM_IN_WORK.has(v.state));
  const ofFilm = (task: Task) => task.batch !== null && filmIds.has(task.batch.id);
  // A finished task of a film that is no longer on «Video» is history too: the film's result
  // is in the library, and its old stages say nothing about now.
  const shown = tasks.filter(
    (task) => !ofFilm(task) && !(task.batch !== null && FINISHED.has(task.state)),
  );
  const videoById = useMemo(() => new Map(videos.map((v) => [v.id, v])), [videos]);

  // Waiting tasks in the order they will run, not the order they appear in the list:
  // otherwise the queue numbers would not match what the core actually does.
  const queued = useMemo(
    () =>
      tasks.filter((task) => task.state === "queued").sort((a, b) => a.queue_order - b.queue_order),
    [tasks],
  );

  /** How a waiting task is named in the queue: a film's by the film and its stage. */
  const queueLabel = (task: Task): string => {
    const film = task.batch ? videoById.get(task.batch.id) : undefined;
    if (film) return `${film.title} · ${filmStage(film, t)}`;
    return t.ui.tasks.kinds[task.kind as TaskKind] ?? task.kind;
  };

  // What is happening right now, in two numbers.
  //
  // A paused task is counted as running rather than waiting: it is holding a place on the
  // machine and its half-made file, and calling it "waiting" would say the opposite of what
  // it is. Cancelled and finished ones are counted as neither.
  const running = tasks.filter(
    (task) => task.state === "running" || task.state === "paused",
  ).length;

  /**
   * The batches with something still to stop.
   *
   * **Counted, not named after one of its films.** The label belongs to the task — it is the
   * film — while the identifier belongs to the batch. The first shape of this took one film's
   * label as the whole batch's heading, so a season of ten was headed by whichever episode
   * happened to come first in the list. The films are named on their own rows; the heading
   * says how big the batch is and what is left of it.
   *
   * A batch that is over gets no heading: a button that does nothing teaches people the
   * application is broken.
   */
  const batches = [
    ...tasks
      // A film on «Video» is a batch of one, of its own stages: «Batch: 1 video» over each
      // film said nothing (T710). Batches of several films (T445) are headed as before.
      .filter((task) => task.batch && !FINISHED.has(task.state) && !ofFilm(task))
      .reduce((seen, task) => {
        const at = seen.get(task.batch!.id) ?? { id: task.batch!.id, films: new Set(), left: 0 };
        at.films.add(task.batch!.label);
        at.left += 1;
        seen.set(at.id, at);
        return seen;
      }, new Map<string, { id: string; films: Set<string>; left: number }>())
      .values(),
  ];

  if (loading) return <div className="panel">{t.ui.tasks.reading}</div>;

  return (
    <div className="panel">
      <h1>{t.ui.tasks.heading}</h1>
      <p className="muted" data-testid="task-counts">
        {fill(t.ui.tasks.counts, { running, queued: queued.length }, t, lang)}
      </p>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <CloseConsequences items={onClose} />

      {/* One button for a whole batch. Without it, stopping ten films means thirty presses —
          and the tenth would start while somebody was still pressing. */}
      {batches.map((batch) => (
        <p key={batch.id} className="hint" data-testid={`batch-${batch.id}`}>
          {fill(t.ui.tasks.batchIs, { films: batch.films.size, left: batch.left }, t, lang)}{" "}
          <button
            type="button"
            className="button-link"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                const stopped = await ipc.tasksCancelBatch(batch.id);
                setStoppedCount(stopped);
              })
            }
          >
            {t.ui.tasks.batchStop}
          </button>
        </p>
      ))}
      {stoppedCount !== null && (
        <p role="status" data-testid="batch-stopped">
          {fill(t.ui.tasks.batchStopped, { n: stoppedCount }, t, lang)}
        </p>
      )}

      <QueueOrder
        queued={queued}
        busy={busy}
        labelOf={queueLabel}
        onReorder={(ids) => void act(async () => void (await ipc.tasksReorder(ids)))}
      />

      {films.length > 0 && (
        <ul className="task-list" data-testid="films">
          {films.map((film) => (
            <FilmRow
              key={film.id}
              film={film}
              busy={busy}
              onPause={() => void act(async () => void (await ipc.videoPause(film.id)))}
              onResume={() => void act(async () => void (await ipc.videoResume(film.id)))}
            />
          ))}
        </ul>
      )}

      {shown.length === 0 && films.length === 0 ? (
        <p className="muted">{t.ui.tasks.empty}</p>
      ) : (
        <ul className="task-list">
          {shown.map((task) => (
            <li key={task.id} className={`task task--${task.state}`}>
              <div className="task__head">
                {/* Which film this is (T445). Thirty rows saying "measuring quality" are a
                    wall, and "stop this one" is then a guess. The label rides on the task
                    itself, so it is still right after a restart and after the file has been
                    renamed in the library. */}
                {task.batch && <span className="task__batch">{task.batch.label}</span>}
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
                  <div className="progress__fill" style={{ width: `${task.progress * 100}%` }} />
                </div>
              )}

              <div className="task__meta">
                {task.stage && <span>{renderStage(task.stage, t, lang)}</span>}
                {formatSpeed(task.speed_bps, t, lang) && (
                  <span>{formatSpeed(task.speed_bps, t, lang)}</span>
                )}
                {formatEta(task.eta_s, t, lang) && <span>{formatEta(task.eta_s, t, lang)}</span>}
              </div>

              {/* One line, the advice folded (T519, T674). */}
              {task.error && <TaskError error={task.error} />}

              {/* What the task worked out and is not a failure (T416): variants taken from
                  a previous run, a measurement that stopped short of the grid, the graphics
                  card that refused. Folded (T674): they explain, and an explanation is one
                  click away rather than in the way. */}
              {task.notices.length > 0 && (
                <details className="task__notices-fold">
                  <summary>{fill(t.ui.tasks.notes, { n: task.notices.length }, t, lang)}</summary>
                  <ul className="task__notices" data-testid="task-notices">
                    {task.notices.map((notice, i) => (
                      <li key={i} role="note">
                        {renderDetail(notice, t, lang)}
                      </li>
                    ))}
                  </ul>
                </details>
              )}

              {/* What the task produced, for a person to go and look at (T519(3)). Only
                  when `result` is not null — an upload or a build_ladder that finished
                  without its medium known (or any other kind) leaves it null, and no
                  link belongs where there is nowhere to send anyone. The library screen
                  cannot yet jump to or highlight one particular medium by id, so this is
                  plain navigation to the section as a whole rather than a promise this
                  screen cannot keep. */}
              {task.result !== null && (
                <p className="task__result">
                  <Link to="/library">{t.ui.tasks.viewResult}</Link>
                </p>
              )}

              <div className="task__actions">
                {task.state === "running" && (
                  <button onClick={() => void act(() => ipc.taskPause(task.id))}>
                    {t.ui.tasks.pause}
                  </button>
                )}
                {/* Only where it would do something (T515). A paused task that nothing raised back
                    into the engine answers "task not found", which describes neither what the
                    person sees nor what they did. */}
                {task.state === "paused" && task.can_resume && (
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
