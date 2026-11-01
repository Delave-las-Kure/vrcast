import { useState } from "react";
import type { AppError, MediaView } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { ErrorNotice } from "../shared/ErrorNotice";

/**
 * T693 (the owner's decision А1 of 2026-10-09) — a set keeps only its segments on the server;
 * the prepared mp4 files of each rung are removed once the set is checked. A set built before
 * that kept them for good, doubling what it takes. They are shown here as one line with the
 * room they take and «Remove» — **removed only on this press** (T577, part b), never by
 * themselves.
 *
 * ⚠ **Asked first, like every other removal on this screen** (tour 0.3.0, E02–E04). A medium
 * and a file both ask before they go; these went on one press. The question says how many
 * and that the set is not touched — the one thing a person would be afraid of.
 *
 * Not shown while a video builds the set (`set_work`): there they are what the build cuts
 * from, and the core would refuse anyway.
 */
export function LeftoverRungFiles({
  serverId,
  media,
  disabled,
  onRemoved,
}: {
  serverId: string;
  media: MediaView;
  disabled?: boolean;
  onRemoved: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const files = (media.set_files ?? []).filter((f) => f.exists_on_server);
  if (files.length === 0 || media.set_work) return null;
  const bytes = files.reduce((sum, f) => sum + f.size_bytes, 0);
  const l = t.ui.library;

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      await ipc.mediaRemoveSetFiles(serverId, media.id);
      setAsking(false);
      onRemoved();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="media__leftover" data-testid={`leftover-${media.id}`}>
      <span className="muted">{fill(l.leftoverMp4, { bytes }, t, lang)}</span>{" "}
      {!asking && (
        <button onClick={() => setAsking(true)} disabled={disabled || busy}>
          {l.leftoverRemove}
        </button>
      )}
      {asking && (
        <div
          className="dialog"
          role="alertdialog"
          aria-label={fill(l.leftoverAsk, { n: files.length }, t, lang)}
          data-testid={`leftover-ask-${media.id}`}
        >
          <h3>{fill(l.leftoverAsk, { n: files.length }, t, lang)}</h3>
          <p>{l.leftoverAskKeeps}</p>
          <div className="form__actions">
            <button type="button" onClick={() => setAsking(false)} disabled={busy} autoFocus>
              {l.deleteNo}
            </button>
            <button
              type="button"
              className="button--danger"
              onClick={() => void remove()}
              disabled={disabled || busy}
            >
              {busy ? l.deleting : l.deleteYes}
            </button>
          </div>
        </div>
      )}
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
    </div>
  );
}
