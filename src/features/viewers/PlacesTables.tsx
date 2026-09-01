/**
 * T461 — whether the tables that turn an address into a place are there, and a way to fetch
 * them.
 *
 * **The gap this closes.** A viewer's country and city are shown on this screen, and they
 * come from tables the application downloads. `geo_status` and `geo_update` were written,
 * registered as commands and described in the contract — and wired to nothing. So when the
 * tables were missing or a month out of date, every viewer simply showed no place at all,
 * and there was nothing on the screen to press. The capability existed and could not be
 * reached; found by the command comparison on 2026-08-28 (T377).
 *
 * **Silent when there is nothing to say.** Tables in place and current is the ordinary state,
 * and a line reporting it on every visit would be noise that teaches people to skip the whole
 * region of the screen — including the times it does matter.
 */

import { useEffect, useState } from "react";

import type { AppError, GeoStatus } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

export function PlacesTables() {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.viewers;
  const [status, setStatus] = useState<GeoStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .geoStatus()
      .then((s) => {
        if (alive) setStatus(s);
      })
      // Not being able to ask about the tables is not a reason to spoil the screen the
      // viewers are on: this is an aside, and the viewers are the point.
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  if (!status) return null;
  if (status.ready && !status.stale) return null;

  const fetchThem = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await ipc.geoUpdate());
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <p className="hint" role="status" data-testid="places-tables">
      {status.ready
        ? fill(words.placesStale, { month: status.month ?? "" }, t, lang)
        : words.placesMissing}{" "}
      <button
        type="button"
        className="button-link"
        onClick={() => void fetchThem()}
        disabled={busy}
      >
        {busy ? words.placesFetching : words.placesFetch}
      </button>
      {error && <span className="form__hint">{words.placesFailed}</span>}
    </p>
  );
}
