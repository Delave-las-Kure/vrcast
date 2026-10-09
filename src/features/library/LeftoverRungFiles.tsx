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
  const [error, setError] = useState<AppError | null>(null);

  const files = (media.set_files ?? []).filter((f) => f.exists_on_server);
  if (files.length === 0 || media.set_work) return null;
  const bytes = files.reduce((sum, f) => sum + f.size_bytes, 0);

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      await ipc.mediaRemoveSetFiles(serverId, media.id);
      onRemoved();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="media__leftover" data-testid={`leftover-${media.id}`}>
      <span className="muted">{fill(t.ui.library.leftoverMp4, { bytes }, t, lang)}</span>{" "}
      <button onClick={() => void remove()} disabled={disabled || busy}>
        {t.ui.library.leftoverRemove}
      </button>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
    </div>
  );
}
