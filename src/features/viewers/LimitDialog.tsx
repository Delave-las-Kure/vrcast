/**
 * T217 — capping one viewer's quality, from the list of viewers (SC-006).
 *
 * **The warnings are shown before the change and have to be agreed to** (FR-066). What is
 * being edited is the configuration of the thing serving somebody's film at that moment,
 * and the three things worth knowing here cannot be undone by knowing them afterwards: that
 * the address may belong to more than one person, that it may stop belonging to this one,
 * and that the cap is below anything that exists.
 *
 * Three actions and no more, as SC-006 asks: choose the medium, set the cap, agree.
 */

import { useEffect, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import { ipc } from "../../shared/ipc";
import type { AppError, LimitPreview } from "../../shared/contract";

function mbps(bps: number): string {
  return `${(bps / 1_000_000).toFixed(1)}`;
}

export function LimitDialog({
  serverId,
  ip,
  media,
  onDone,
  onCancel,
}: {
  serverId: string;
  ip: string;
  /** What the library holds, so the person picks rather than types. */
  media: { slug: string; title: string }[];
  onDone?: () => void;
  onCancel?: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.limits;

  const [slug, setSlug] = useState(media[0]?.slug ?? "");
  const [capMbps, setCapMbps] = useState(6);
  const [preview, setPreview] = useState<LimitPreview | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [applying, setApplying] = useState(false);

  // What this would do, asked again on every change. Nothing is altered by asking, and a
  // person choosing a cap is choosing from what they can see it will leave.
  useEffect(() => {
    if (!slug) return;
    let alive = true;
    setError(null);
    ipc
      .limitPreview({ server_id: serverId, ip, slug, cap_bps: capMbps * 1_000_000 })
      .then((answer) => {
        if (alive) setPreview(answer);
      })
      .catch((e: AppError) => {
        if (alive) {
          setPreview(null);
          setError(e);
        }
      });
    return () => {
      alive = false;
    };
  }, [serverId, ip, slug, capMbps]);

  return (
    <section aria-label={words.title}>
      <h3>{words.title}</h3>
      <p>{words.explain}</p>

      <label>
        {words.pickMedia}
        <select value={slug} onChange={(e) => setSlug(e.target.value)}>
          {media.map((m) => (
            <option key={m.slug} value={m.slug}>
              {m.title}
            </option>
          ))}
        </select>
      </label>

      <label>
        {words.cap}
        <input
          type="number"
          min={1}
          value={capMbps}
          onChange={(e) => setCapMbps(Math.max(1, Number(e.target.value)))}
        />
      </label>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {preview && (
        <>
          <p>{words.willGet}</p>
          <ul data-testid="kept">
            {preview.kept.map((v) => (
              <li key={v.path}>
                {mbps(v.bandwidth)} Mbit/s — {v.width}×{v.height}
              </li>
            ))}
          </ul>

          {/*
            Every warning, before the button rather than after it. A warning shown
            afterwards is a report, and a report about something already done is of no use
            to anybody.
          */}
          <ul role="alert" data-testid="warnings">
            {preview.warnings.map((w, i) => (
              <li key={i}>{renderDetail(w, t, lang)}</li>
            ))}
          </ul>
        </>
      )}

      <button
        type="button"
        data-testid="confirm"
        disabled={!preview || applying}
        onClick={() => {
          setApplying(true);
          ipc
            .limitSet({ server_id: serverId, ip, slug, cap_bps: capMbps * 1_000_000 }, true)
            .then(() => onDone?.())
            .catch((e: AppError) => setError(e))
            .finally(() => setApplying(false));
        }}
      >
        {words.confirm}
      </button>
      <button type="button" onClick={() => onCancel?.()}>
        {words.cancel}
      </button>
    </section>
  );
}
