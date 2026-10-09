/**
 * T218 — what is capped right now, and taking a cap off (FR-064, FR-065).
 *
 * **Read from the server, never from a note kept here.** A note goes stale the hour
 * somebody edits the server by hand, and a list of limits that does not match the server is
 * worse than no list: it tells a person their viewer is capped when they are not, and they
 * spend the evening looking for the fault somewhere else.
 */

import { useCallback, useEffect, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { formatBitrate } from "../../shared/i18n/format";
import { ipc } from "../../shared/ipc";
import type { AppError, QualityLimit } from "../../shared/contract";

/** «2026-08-26 13:00», in this machine's time — not the raw stamp the rule carries (T704). */
function when(stamp: string): string {
  const at = new Date(stamp);
  if (Number.isNaN(at.getTime())) return stamp;
  const two = (n: number) => String(n).padStart(2, "0");
  return `${at.getFullYear()}-${two(at.getMonth() + 1)}-${two(at.getDate())} ${two(at.getHours())}:${two(at.getMinutes())}`;
}

export function LimitsList({ serverId }: { serverId: string }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.limits;

  const [limits, setLimits] = useState<QualityLimit[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [lifting, setLifting] = useState<string | null>(null);
  // The films' own names, so a rule reads «Фильм с двумя дорожками» and not its directory.
  // A library that will not load leaves the directory names: still true, just plainer.
  const [titles, setTitles] = useState<Record<string, string>>({});

  useEffect(() => {
    let alive = true;
    ipc
      .libraryList(serverId)
      .then((view) => {
        if (alive) setTitles(Object.fromEntries(view.media.map((m) => [m.slug, m.title])));
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [serverId]);

  const reload = useCallback(() => {
    ipc
      .limitsList(serverId)
      .then(setLimits)
      .catch((e: AppError) => setError(e));
  }, [serverId]);

  useEffect(reload, [reload]);

  return (
    <section aria-label={words.listTitle}>
      <h3>{words.listTitle}</h3>
      <p className="hint" data-testid="limits-when">
        {words.listHint}
      </p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {limits && limits.length === 0 && <p data-testid="no-limits">{words.listEmpty}</p>}

      {limits && limits.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>{words.columnWho}</th>
              <th>{words.columnMedia}</th>
              <th>{words.columnCap}</th>
              <th>{words.columnSince}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {limits.map((limit) => {
              const key = `${limit.ip}/${limit.slug}`;
              return (
                <tr key={key} data-testid={`limit-${key}`}>
                  <td>{limit.ip}</td>
                  <td>{titles[limit.slug] ?? limit.slug}</td>
                  <td>{formatBitrate(limit.cap_bps, lang)}</td>
                  <td>{when(limit.set_at)}</td>
                  <td>
                    <button
                      type="button"
                      disabled={lifting === key}
                      onClick={() => {
                        setLifting(key);
                        ipc
                          .limitClear(serverId, limit.ip, limit.slug)
                          // Reloaded from the server rather than struck off the list here:
                          // what we think happened and what happened are two different
                          // things, and this is the moment they part company.
                          .then(reload)
                          .catch((e: AppError) => setError(e))
                          .finally(() => setLifting(null));
                      }}
                    >
                      {lifting === key ? words.removing : words.remove}
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}
