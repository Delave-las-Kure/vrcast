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
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type { AppError, QualityLimit } from "../../shared/contract";

function mbps(bps: number): string {
  return `${(bps / 1_000_000).toFixed(1)} Mbit/s`;
}

export function LimitsList({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.limits;

  const [limits, setLimits] = useState<QualityLimit[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [lifting, setLifting] = useState<string | null>(null);

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
      <p>{words.listFromServer}</p>

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
                  <td>{limit.slug}</td>
                  <td>{mbps(limit.cap_bps)}</td>
                  <td>{limit.set_at}</td>
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
