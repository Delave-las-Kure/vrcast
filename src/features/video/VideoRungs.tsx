/**
 * T673 — the rung editor for one video, opened from its card.
 *
 * The same `RungEditor` the quality screen used, on the video's own plan: what is saved goes
 * straight to the video (`videoSetRungs`), and a video that stopped on a problem carries on
 * from where it stopped (`videoRetry`). Nothing is carried between screens by hand.
 */

import { useMemo, useState } from "react";

import type { AppError, Rung, VideoView } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { RungEditor } from "../ladder/RungEditor";
import { ErrorNotice } from "../shared/ErrorNotice";
import { factsOf } from "./rules";

export function VideoRungs({
  video,
  onSaved,
  onClose,
}: {
  video: VideoView;
  onSaved: (v: VideoView) => void;
  onClose: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const w = t.ui.video;
  const [rungs, setRungs] = useState<Rung[]>(video.plan?.rungs ?? []);
  const [leftOut, setLeftOut] = useState<ReadonlySet<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  // Memoised: the editor re-checks the rungs whenever this object changes identity.
  const source = useMemo(() => (video.source ? factsOf(video.source) : null), [video.source]);

  if (!source) return null;

  const save = async (chosen: Rung[] | null) => {
    setBusy(true);
    setError(null);
    try {
      let next = await ipc.videoSetRungs(video.id, chosen);
      if (next.state === "problem") next = await ipc.videoRetry(video.id, false);
      onSaved(next);
      onClose();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const kept = rungs.filter((r) => !leftOut.has(r.index));

  return (
    <div className="dialog video__rungs" role="dialog" aria-label={w.rungs}>
      <h3>{fill(w.editorTitle, { title: video.title }, t, lang)}</h3>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      <RungEditor
        rungs={rungs}
        source={source}
        onChange={setRungs}
        left_out={leftOut}
        onToggle={(index) =>
          setLeftOut((was) => {
            const next = new Set(was);
            if (!next.delete(index)) next.add(index);
            return next;
          })
        }
      />
      <div className="form__actions">
        <button type="button" onClick={onClose} disabled={busy}>
          {t.ui.common.cancel}
        </button>
        <button type="button" onClick={() => void save(null)} disabled={busy}>
          {w.resetRungs}
        </button>
        <button
          type="button"
          className="button--primary"
          onClick={() => void save(kept)}
          disabled={busy || kept.length === 0}
        >
          {w.saveRungs}
        </button>
      </div>
    </div>
  );
}
