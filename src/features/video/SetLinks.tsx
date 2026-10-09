/**
 * T694 (the owner's decisions of 2026-10-09) — the links to a finished set, the same
 * on the «Done» card and in the library.
 *
 * «Auto» is the set's master: the player picks the quality itself. Beside it one link per
 * quality, the rung's own playlist — when everybody's connection is slow, a steady low bitrate
 * serves better than the player's choosing. Each has «Copy»; with a CDN, «Via CDN» too, and
 * one line saying what a CDN hides. Under it, folded, the one thing friends must do in VRChat.
 * No addresses on view and no file names: what is copied is what matters.
 */

import { useEffect, useRef, useState } from "react";

import type { QualityLink } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { megabits } from "./rules";

export function SetLinks({
  auto,
  qualities,
}: {
  /** The set's master, as `Links`/`LadderSetView` carry it. */
  auto: { origin: string; cdn: string | null };
  qualities: QualityLink[];
}) {
  const t = useT();
  const { lang } = useLang();
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

  const rows = [
    { key: "auto", label: w.linkAuto, origin: auto.origin, cdn: auto.cdn },
    ...qualities.map((q) => ({
      key: q.origin,
      label: fill(w.rungLine, { height: q.height, mbps: megabits(q.bitrate_bps, lang) }, t, lang),
      origin: q.origin,
      cdn: q.cdn,
    })),
  ];

  return (
    <div className="set-links" data-testid="set-links">
      <ul className="set-links__list">
        {rows.map((r) => (
          <li key={r.key} className="set-links__row" data-testid="set-link">
            <span className="set-links__label">{r.label}</span>
            <button type="button" onClick={() => void copy(r.origin)}>
              {w.copy}
            </button>
            {r.cdn && (
              <button type="button" onClick={() => void copy(r.cdn!)}>
                {w.linkViaCdn}
              </button>
            )}
          </li>
        ))}
      </ul>
      {said && (
        <p className="set-links__said" role="status">
          {said}
        </p>
      )}
      {auto.cdn && <p className="muted set-links__cdn">{w.cdnBlind}</p>}
      <details className="error-more set-links__help">
        <summary>{w.friendsHow}</summary>
        <p>{w.friendsHowText}</p>
      </details>
    </div>
  );
}
