/**
 * T673 — one video, from its plan to its link.
 *
 * Before «Start» the card shows the plan; after it, the bar of stages with the current one
 * lit. A problem is one line, folded details, and the buttons the core said would help.
 * Short labels only (T674): whatever needs explaining lives in the error's «Details».
 */

import { useEffect, useRef, useState } from "react";

import type {
  AppError,
  Links,
  VideoPlan,
  VideoProblemAction,
  VideoView,
} from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { formatDuration } from "../../shared/i18n/format";
import { fill, renderDetail } from "../../shared/i18n/render";
import { ErrorFolded } from "../shared/ErrorNotice";
import { basename } from "../shared/names";
import {
  STAGES,
  canCancel,
  canPause,
  canRemove,
  canResume,
  canSetAudio,
  canSetName,
  canSetRungs,
  canStart,
  megabits,
  showsPlan,
  trackLabel,
} from "./rules";
import { VideoRungs } from "./VideoRungs";

export function VideoCard({
  video,
  failure,
  onChanged,
  onRemoved,
  onFailed,
}: {
  video: VideoView;
  /** The last action on this card that the core refused. */
  failure: AppError | null;
  onChanged: (v: VideoView) => void;
  onRemoved: (id: string) => void;
  onFailed: (id: string, error: AppError | null) => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const w = t.ui.video;
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [naming, setNaming] = useState(false);
  const [title, setTitle] = useState(video.title);
  /** «Replace» (T676): the one-line question, then — if somebody is watching — «anyway». */
  const [replacing, setReplacing] = useState<"ask" | "viewers" | null>(null);

  const run = async (act: () => Promise<VideoView | void>) => {
    setBusy(true);
    onFailed(video.id, null);
    try {
      const next = await act();
      if (next) onChanged(next);
    } catch (e) {
      onFailed(video.id, toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const id = video.id;
  const rename = (retry: boolean) =>
    run(async () => {
      const named = await ipc.videoSetName(id, title.trim(), null);
      setNaming(false);
      return retry ? ipc.videoRetry(id, false) : named;
    });

  const replace = (confirmed: boolean) =>
    run(async () => {
      try {
        const next = await ipc.videoReplace(id, confirmed);
        setReplacing(null);
        return next;
      } catch (e) {
        // Somebody is watching (as T571): nothing was removed, «anyway» is the way on.
        setReplacing(toAppError(e).code === "FILE_IN_USE" ? "viewers" : null);
        throw e;
      }
    });

  const doAction = (a: VideoProblemAction) => {
    if (a === "retry") void run(() => ipc.videoRetry(id, false));
    else if (a === "build_anyway") void run(() => ipc.videoRetry(id, true));
    else if (a === "replace") setReplacing("ask");
    else if (a === "edit_rungs") setEditing(true);
    else {
      setTitle(video.title);
      setNaming(true);
    }
  };

  const actionLabel: Record<VideoProblemAction, string> = {
    retry: w.retry,
    build_anyway: w.buildAnyway,
    edit_rungs: w.rungs,
    replace: w.replace,
    rename: w.retry,
  };

  const planShown = showsPlan(video);
  const fileName = basename(video.source_path);
  const tracks = video.source?.audio_tracks ?? [];
  const problemActions = video.state === "problem" ? (video.problem?.actions ?? []) : [];
  // «Rename» puts the name field up and its own «Retry» beside it; the field is the action.
  const renameOffered = problemActions.includes("rename") && canSetName(video);
  const confirming = replacing !== null && problemActions.includes("replace");

  return (
    <li className={`video video--${video.state}`} data-testid={`video-${video.id}`}>
      <div className="video__head">
        {naming ? (
          <form
            className="form__inline"
            onSubmit={(e) => {
              e.preventDefault();
              void rename(video.state === "problem");
            }}
          >
            <input
              aria-label={w.title}
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              autoFocus
            />
            <button type="submit" disabled={busy || title.trim() === ""}>
              {video.state === "problem" ? w.retry : w.saveTitle}
            </button>
            <button type="button" onClick={() => setNaming(false)} disabled={busy}>
              {t.ui.common.cancel}
            </button>
          </form>
        ) : (
          <>
            <span className="video__title">{video.title}</span>
            {canSetName(video) && !renameOffered && (
              <button
                type="button"
                className="button--quiet"
                onClick={() => {
                  setTitle(video.title);
                  setNaming(true);
                }}
                disabled={busy}
              >
                {w.editTitle}
              </button>
            )}
          </>
        )}
      </div>
      <p className="video__file muted">{fileName}</p>

      {planShown && video.state === "planning" && !video.plan && (
        <p className="muted" role="status">
          {video.start_requested ? w.startsAfterPlan : w.planning}
          {/* T688 — waiting for a place for its trial encodes among the heavy work. */}
          {video.progress?.task_state === "queued" && ` · ${w.queued}`}
        </p>
      )}
      {planShown && video.plan && <Plan plan={video.plan} slug={video.slug} t={t} lang={lang} />}

      {planShown && tracks.length > 1 && (
        <label className="video__audio">
          <span>{w.audio}</span>
          <select
            value={video.audio_track}
            disabled={busy || !canSetAudio(video)}
            onChange={(e) => void run(() => ipc.videoSetAudio(id, Number(e.target.value)))}
          >
            {tracks.map((track) => (
              <option key={track.index} value={track.index}>
                {trackLabel(track, t, lang)}
              </option>
            ))}
          </select>
        </label>
      )}

      {!planShown && <StageBar video={video} t={t} lang={lang} />}

      {video.state === "done" && video.link && <LinkCopy link={video.link} />}

      {video.state === "problem" && video.problem && (
        <div className="video__problem" role="alert">
          <ErrorFolded error={video.problem.error} lineClassName="video__problem-line" />
        </div>
      )}

      {failure && (
        <div className="video__problem" role="alert">
          <ErrorFolded error={failure} lineClassName="video__problem-line" />
        </div>
      )}

      {confirming && (
        <div className="video__confirm" role="group" aria-label={w.replace}>
          {replacing === "ask" && <p>{fill(w.replaceAsk, { title: video.title }, t, lang)}</p>}
          <button
            type="button"
            className="button--danger"
            disabled={busy}
            onClick={() => void replace(replacing === "viewers")}
          >
            {replacing === "viewers" ? w.replaceAnyway : w.replace}
          </button>
          <button type="button" disabled={busy} onClick={() => setReplacing(null)}>
            {t.ui.common.cancel}
          </button>
        </div>
      )}

      {!naming && !confirming && (
        <div className="video__actions">
          {canStart(video) && (
            <button
              type="button"
              className="button--primary"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  const [answer] = await ipc.videoStart([id]);
                  if (answer?.error) throw answer.error;
                })
              }
            >
              {w.start}
            </button>
          )}
          {problemActions.map((a) =>
            a === "rename" && !renameOffered ? null : (
              <button key={a} type="button" disabled={busy} onClick={() => doAction(a)}>
                {actionLabel[a]}
              </button>
            ),
          )}
          {video.state === "cancelled" && (
            <button type="button" disabled={busy} onClick={() => doAction("retry")}>
              {w.retry}
            </button>
          )}
          {canPause(video) && (
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(() => ipc.videoPause(id))}
            >
              {w.pause}
            </button>
          )}
          {canResume(video) && (
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(() => ipc.videoResume(id))}
            >
              {w.resume}
            </button>
          )}
          {canSetRungs(video) && video.state !== "problem" && (
            <button type="button" disabled={busy} onClick={() => setEditing(true)}>
              {w.rungs}
            </button>
          )}
          {/* A plan that failed has nothing to stop: «Remove» is the way out of it. */}
          {canCancel(video) && !(video.stage === "planned" && video.state === "problem") && (
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(() => ipc.videoCancel(id))}
            >
              {w.cancel}
            </button>
          )}
          {canRemove(video) && (
            <button
              type="button"
              className="button--quiet"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  // With its work alive the core stops it first (T683): the card stays,
                  // «stopping», until `video:removed`.
                  const left = await ipc.videoRemove(id);
                  if (left) return left;
                  onRemoved(id);
                })
              }
            >
              {w.remove}
            </button>
          )}
        </div>
      )}

      {editing && canSetRungs(video) && (
        <VideoRungs video={video} onSaved={onChanged} onClose={() => setEditing(false)} />
      )}
    </li>
  );
}

/** The plan before «Start»: rungs, size, time, and anything that would stop it. */
function Plan({
  plan,
  slug,
  t,
  lang,
}: {
  plan: VideoPlan;
  slug: string;
  t: Catalogue;
  lang: Lang;
}) {
  const w = t.ui.video;
  const seconds = plan.measure_s + (plan.encode_s ?? 0);
  const minutes = seconds > 0 ? Math.max(1, Math.round(seconds / 60)) : null;
  // T689 — what the time covers (measuring and encoding; not sending, cutting or checking,
  // which nothing here can reckon), and whether it is a guess of the formula or the model.
  const covers = plan.measure_s > 0 ? w.measureAndEncodeTime : w.encodeTime;
  const preliminary = plan.from === "formula" || plan.encode_estimate === "model";
  const folded = [...plan.objections, ...plan.notices];

  return (
    <div className="video__plan" data-testid="plan">
      <ul className="video__rung-list">
        {plan.rungs.map((r) => (
          <li key={r.index}>
            {fill(w.rungLine, { height: r.height, mbps: megabits(r.bitrate_bps, lang) }, t, lang)}
            {/* T680 — a rung not measured yet is measured first when «Start» is pressed. */}
            {plan.from === "edited" &&
              r.quality.state === "not_measured" &&
              ` · ${w.rungToMeasure}`}
          </li>
        ))}
      </ul>
      <p className="video__facts muted">
        <span>{fill(w.onServer, { bytes: plan.server_bytes }, t, lang)}</span>
        {minutes !== null && (
          <span>
            {fill(w.aboutMinutes, { what: covers, n: minutes }, t, lang)}
            {preliminary && ` · ${w.preliminary}`}
          </span>
        )}
      </p>
      {plan.server_space.state === "short" && (
        <p className="video__warn">
          {fill(w.shortServer, { bytes: plan.server_space.short_by }, t, lang)}
        </p>
      )}
      {plan.local_space.state === "short" && (
        <p className="video__warn">
          {fill(w.shortLocal, { bytes: plan.local_space.short_by }, t, lang)}
        </p>
      )}
      {plan.name_taken === true && (
        <p className="video__warn">{fill(w.nameTaken, { slug }, t, lang)}</p>
      )}
      {folded.length > 0 && (
        <details className="error-more">
          <summary>
            {plan.objections.length > 0
              ? fill(w.objections, { n: plan.objections.length }, t, lang)
              : t.ui.common.more}
          </summary>
          <ul>
            {folded.map((d, i) => (
              <li key={i}>{renderDetail(d, t, lang)}</li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

/** Measure → Encode → Upload → Cut → Check → Done, with the current one lit. */
function StageBar({ video, t, lang }: { video: VideoView; t: Catalogue; lang: Lang }) {
  const w = t.ui.video;
  // Started and not yet at its first stage reads as the first stage beginning.
  const at =
    video.state === "done"
      ? STAGES.length
      : Math.max(0, STAGES.indexOf(video.stage as (typeof STAGES)[number]));
  const p = video.progress;
  // T689 — a stage that cannot say how far it has got (the check asks every rung and answers
  // once) is shown without a figure, not as nearly done or as nothing done.
  const known = !(video.stage === "verifying");

  const facts: string[] = [];
  if (video.state === "cancelling") facts.push(w.stopping);
  else if (video.state === "cancelled") facts.push(w.cancelled);
  else if (video.state === "paused") facts.push(w.paused);
  else if (p?.task_state === "queued") facts.push(w.queued);
  if (p && video.state !== "done" && video.state !== "cancelled") {
    if (p.task_state !== "queued" && known) facts.push(`${Math.round(p.progress * 100)}%`);
    if (p.speed_bps) facts.push(fill(w.speed, { bytes: p.speed_bps }, t, lang));
    if (p.eta_s) facts.push(fill(w.left, { time: formatDuration(p.eta_s) }, t, lang));
    if (p.rung !== null) facts.push(fill(w.rungOf, { k: p.rung, n: p.rungs }, t, lang));
  }

  return (
    <div className="video__progress">
      <ol className="video__stages" aria-label={w.stagesLabel}>
        {STAGES.map((stage, i) => {
          const mark = i < at ? "passed" : i === at ? "current" : "ahead";
          return (
            <li
              key={stage}
              className={`video__stage video__stage--${mark}`}
              data-stage={stage}
              data-mark={mark}
              aria-current={mark === "current" ? "step" : undefined}
            >
              {mark === "passed" ? "✓ " : ""}
              {w.stages[stage]}
            </li>
          );
        })}
      </ol>
      {p && video.state === "working" && p.task_state !== "queued" && (
        <div
          className="progress"
          role="progressbar"
          aria-valuenow={known ? Math.round(p.progress * 100) : undefined}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-busy={known ? undefined : true}
          aria-label={w.stages[STAGES[Math.min(at, STAGES.length - 1)]]}
        >
          {known && <div className="progress__fill" style={{ width: `${p.progress * 100}%` }} />}
        </div>
      )}
      {facts.length > 0 && (
        <p className="video__facts muted" data-testid="stage-facts">
          {facts.map((f) => (
            <span key={f}>{f}</span>
          ))}
        </p>
      )}
    </div>
  );
}

/** The link to the finished set, and a button that copies it. */
function LinkCopy({ link }: { link: Links }) {
  const t = useT();
  const w = t.ui.video;
  const [said, setSaid] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const copy = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      setSaid(w.copied);
    } catch {
      setSaid(w.copyFailed);
    }
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setSaid(null), 2000);
  };

  return (
    <div className="video__link">
      <a href={link.origin} target="_blank" rel="noreferrer">
        {link.origin}
      </a>
      <button type="button" onClick={() => void copy(link.origin)}>
        {w.copy}
      </button>
      {link.cdn && (
        <button type="button" onClick={() => void copy(link.cdn!)}>
          {w.copyCdn}
        </button>
      )}
      {said && <span role="status">{said}</span>}
    </div>
  );
}
