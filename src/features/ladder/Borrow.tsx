/**
 * T427, T428 — taking another film's measurement, and getting back out of it.
 *
 * **A capability with no way in.** The core has been able to lend a measurement since
 * milestone C: `quality_measure_reuse`, `quality_measurements` and `quality_measure_forget`
 * were written, registered as commands and described in the contract — and called from
 * nowhere at all. So the second episode of a season, whose ladder the first episode's
 * measurement answers exactly, had to be measured again from scratch: half an hour of
 * encoding per episode, twelve times a season, for an answer already in the database.
 * Found by the command comparison on 2026-08-28.
 *
 * **And no way out.** The offer to measure disappears the moment any measurement is found,
 * borrowed or not — so a film that took somebody else's had no "measure it properly" and no
 * "forget this". A loan you cannot undo is not a shortcut, it is a decision made once and
 * for good.
 *
 * The core decides what may be lent; this screen never second-guesses it. Where it refuses,
 * it now says which field differed (T431), and that sentence is shown as it comes.
 */

import { useEffect, useState } from "react";

import type { AppError, StoredMeasurement } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { ErrorNotice } from "../shared/ErrorNotice";
import { filmLabel } from "../shared/names";

export function Borrow({
  path,
  /** Set when this film already has a measurement of its own or a borrowed one. */
  borrowedFrom,
  measuredHere,
  sourceKey,
  codec,
  onChanged,
}: {
  path: string;
  borrowedFrom: string | null;
  measuredHere: boolean;
  sourceKey: string | null;
  codec: string;
  onChanged: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.ladder;
  const [donors, setDonors] = useState<StoredMeasurement[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .qualityMeasurements()
      .then((all) => {
        if (alive) setDonors(all);
      })
      // Being unable to list donors is not a reason to spoil the screen: the ladder is the
      // point, and this is an offer beside it.
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [path]);

  const act = async (what: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await what();
      onChanged();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  // A film cannot borrow from itself, and a measurement that is itself borrowed is not
  // offered on: the chain would work — the core follows it to the true donor — but a list
  // of copies of one measurement is a list that hides how few real ones there are.
  const offerable = (donors ?? []).filter(
    (d) => d.source_key !== sourceKey && d.borrowed_from === null,
  );

  return (
    <section aria-label={words.borrowTitle} data-testid="borrow">
      <h3>{words.borrowTitle}</h3>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {/* Where it came from, when it came from somewhere else. Said here as well as on each
          rung (T419): the rung says "borrowed", this says from what. */}
      {borrowedFrom && (
        <p role="note" data-testid="borrowed-from">
          {fill(words.borrowedFromFile, { film: filmLabel(borrowedFrom) }, t, lang)}
        </p>
      )}

      {/* **The way out** (T428). Offered whenever there is a measurement at all, borrowed or
          not: a measurement of one's own can be wrong too — the file was re-encoded, or the
          probe ran on a card it was not calibrated for. Throwing it away is how it is taken
          again, and until now nothing could. */}
      {(borrowedFrom || measuredHere) && sourceKey && (
        <button
          type="button"
          data-testid="forget"
          disabled={busy}
          onClick={() => void act(() => ipc.qualityMeasureForget(sourceKey, codec))}
        >
          {borrowedFrom ? words.forgetBorrowed : words.forgetMeasured}
        </button>
      )}

      {/* Offered only where there is nothing yet. A film with a measurement of its own has no
          business being handed somebody else's without first throwing its own away — and the
          button for that is right above. */}
      {!borrowedFrom && !measuredHere && (
        <>
          <p className="hint">{words.borrowExplain}</p>
          {offerable.length === 0 ? (
            <p className="muted" data-testid="no-donors">
              {words.borrowNothingToTake}
            </p>
          ) : (
            <ul data-testid="donors">
              {offerable.map((d) => (
                <li key={`${d.source_key}:${d.codec}`}>
                  {filmLabel(d.source_path)}{" "}
                  <span className="muted">
                    {fill(
                      words.borrowDonorFacts,
                      { width: d.width, height: d.height, fps: d.fps, anchor: d.anchor_mbps },
                      t,
                      lang,
                    )}
                  </span>{" "}
                  <button
                    type="button"
                    className="button-link"
                    disabled={busy}
                    aria-label={fill(
                      words.borrowFromFilm,
                      { film: filmLabel(d.source_path) },
                      t,
                      lang,
                    )}
                    onClick={() =>
                      void act(() => ipc.qualityMeasureReuse(d.source_key, { path, codec }))
                    }
                  >
                    {words.borrowTake}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
