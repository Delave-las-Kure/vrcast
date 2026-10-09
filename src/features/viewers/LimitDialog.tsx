/**
 * T217 — capping one viewer's quality, from the list of viewers (SC-006).
 *
 * **The warnings are shown before the change and have to be agreed to** (FR-066). What is
 * being edited is the configuration of the thing serving somebody's film at that moment,
 * and the three things worth knowing here cannot be undone by knowing them afterwards: that
 * the address may belong to more than one person, that it may stop belonging to this one,
 * and that the cap is below anything that exists.
 *
 * Three actions and no more, as SC-006 asks: open it on the viewer, set the cap, agree. The
 * medium is the one the viewer is watching (T668) — choosing it again is a fourth action,
 * and choosing wrongly is a cap on a film nobody at that address is watching.
 *
 * ⚠ **T668 (QA-24B-09) — agreeing to exactly what is on screen.** The preview used to stay
 * on screen, and the button stayed live, while a new one was being asked for: change the
 * cap and press at once, and the change went through with `confirmed: true` against the
 * previous cap's rungs and warnings — the "below anything that exists" warning, which
 * depends on the cap, possibly never seen. A preview now belongs to the exact server,
 * address, set and cap it was asked for; anything else on screen is not it, and the button
 * waits for the answer that is.
 *
 * T704 (the owner's decision Г2, QA-26 №2) — **honest about when it works.** The rule
 * changes what the film's description says to this address the next time it is asked for;
 * a player that already has the description keeps the quality it chose. So the dialog says
 * «подействует, когда зритель перезапустит видео» before the button, and the screen says it
 * again once the rule is written. It opens over the screen (modal), on the film this viewer
 * is watching — and while that is not known yet, on nothing rather than on the first film of
 * the catalogue, switching to the viewer's film by itself the moment it becomes known. Only
 * films with a set of qualities are offered: a cap on anything else has nothing to take away.
 */

import { useEffect, useRef, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { formatBitrate, formatResolution } from "../../shared/i18n/format";
import { fill, renderDetail } from "../../shared/i18n/render";
import { ipc } from "../../shared/ipc";
import type { AppError, LimitPreview, LimitRequest } from "../../shared/contract";

/** What a preview was asked for. Two previews answer the same question only if these match. */
function keyOf(request: LimitRequest): string {
  return JSON.stringify([request.server_id, request.ip, request.slug, request.cap_bps]);
}

/** A medium as the dialog offers it. `hasSet: false` — no set of qualities, nothing to cap. */
export type LimitMedium = { slug: string; title: string; hasSet?: boolean };

export function LimitDialog({
  serverId,
  ip,
  media,
  initialSlug,
  onDone,
  onCancel,
}: {
  serverId: string;
  ip: string;
  /** What the library holds, so the person picks rather than types. */
  media: LimitMedium[];
  /**
   * The set the viewer is watching, when that is known (T668). The dialog opens on it, and
   * follows it while the person has not chosen anything themselves (T704): a viewer's film
   * often becomes known only a little after they appear in the list.
   */
  initialSlug?: string | null;
  onDone?: () => void;
  onCancel?: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.limits;

  // Only what can be capped (T704).
  const offered = media.filter((m) => m.hasSet !== false);
  const watching = initialSlug && offered.some((m) => m.slug === initialSlug) ? initialSlug : null;
  // The viewer's film; or the only film there is; or nothing — never "the first of several".
  const fallback = (): string => watching ?? (offered.length === 1 ? offered[0].slug : "");
  const [slug, setSlug] = useState(fallback);
  // Whether the person chose the film themselves: then it is not changed under them.
  const [picked, setPicked] = useState(false);
  const [capMbps, setCapMbps] = useState(6);
  // The preview together with what it was asked for — see the note at the top.
  const [answered, setAnswered] = useState<{ key: string; preview: LimitPreview } | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [applying, setApplying] = useState(false);
  const first = useRef<HTMLSelectElement>(null);

  // The library may still be arriving when the dialog opens, and the viewer's film may become
  // known after it did. Either way the viewer's own film is chosen — unless the person has
  // chosen one themselves.
  useEffect(() => {
    if (picked) return;
    const wanted = fallback();
    if (wanted && wanted !== slug) setSlug(wanted);
    // `fallback` reads only `media` and `initialSlug`, both listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [media, initialSlug, picked, slug]);

  // Over the screen: focus inside, and Escape is "Cancel" — except while the change is going
  // on, when leaving would hide its answer.
  useEffect(() => {
    first.current?.focus();
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !applying) onCancel?.();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [applying, onCancel]);

  const request: LimitRequest = { server_id: serverId, ip, slug, cap_bps: capMbps * 1_000_000 };
  const key = keyOf(request);
  // Only the preview for exactly what is on screen now counts. An older one is not shown as
  // if it were this one, and cannot be agreed to.
  const preview = answered?.key === key ? answered.preview : null;
  const previewing = Boolean(slug) && preview === null && error === null;

  // What this would do, asked again on every change. Nothing is altered by asking, and a
  // person choosing a cap is choosing from what they can see it will leave.
  useEffect(() => {
    if (!slug) return;
    let alive = true;
    const asked: LimitRequest = {
      server_id: serverId,
      ip,
      slug,
      cap_bps: capMbps * 1_000_000,
    };
    setError(null);
    ipc
      .limitPreview(asked)
      .then((answer) => {
        if (alive) setAnswered({ key: keyOf(asked), preview: answer });
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, [serverId, ip, slug, capMbps]);

  return (
    <div className="modal" role="dialog" aria-modal="true" aria-label={words.title}>
      <section className="dialog" data-testid="limit-dialog">
        <h3>
          {words.title} · <code>{ip}</code>
        </h3>

        {offered.length === 0 ? (
          <p className="hint" data-testid="no-sets">
            {words.noSets}
          </p>
        ) : (
          <label>
            <span>{words.pickMedia}</span>
            <select
              ref={first}
              value={slug}
              onChange={(e) => {
                setPicked(true);
                setSlug(e.target.value);
              }}
              disabled={applying}
            >
              {!slug && (
                <option value="" disabled>
                  {words.pickPlaceholder}
                </option>
              )}
              {offered.map((m) => (
                <option key={m.slug} value={m.slug}>
                  {m.title}
                </option>
              ))}
            </select>
          </label>
        )}
        {!watching && offered.length > 1 && (
          <p className="hint" data-testid="watching-unknown">
            {words.watchingUnknown}
          </p>
        )}

        <label>
          <span>{words.cap}</span>
          <input
            type="number"
            min={1}
            value={capMbps}
            disabled={applying}
            onChange={(e) => setCapMbps(Math.max(1, Number(e.target.value)))}
          />
        </label>

        {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

        {previewing && (
          <p className="hint" role="status" data-testid="limit-previewing">
            {words.previewing}
          </p>
        )}

        {preview && (
          <>
            {/*
              Nothing kept and nothing below the lightest either means there were no rungs at
              all: this medium has no quality set, so there is nothing a cap could take away.
              Shown instead of an empty list under a heading promising one — an empty list
              reads as "loading", or as a fault.
            */}
            {preview.kept.length === 0 ? (
              <p data-testid="no-ladder">{words.noLadder}</p>
            ) : (
              <>
                <p>{words.willGet}</p>
                <ul data-testid="kept">
                  {preview.kept.map((v) => (
                    <li key={v.path}>
                      {fill(
                        words.variant,
                        {
                          rate: formatBitrate(v.bandwidth, lang),
                          size: formatResolution(v.width, v.height),
                        },
                        t,
                        lang,
                      )}
                    </li>
                  ))}
                </ul>
              </>
            )}

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

        <p className="hint" data-testid="limit-when">
          {words.whenEffective}
        </p>

        <div className="form__actions">
          <button
            type="button"
            className="button--primary"
            data-testid="confirm"
            disabled={!preview || applying}
            onClick={() => {
              // What goes is what the preview on screen was asked for — the same thing, since
              // a preview for anything else would not be on screen.
              setApplying(true);
              ipc
                .limitSet(request, true)
                .then(() => onDone?.())
                .catch((e: AppError) => setError(e))
                .finally(() => setApplying(false));
            }}
          >
            {applying ? words.applying : words.confirm}
          </button>
          <button type="button" disabled={applying} onClick={() => onCancel?.()}>
            {words.cancel}
          </button>
        </div>
      </section>
    </div>
  );
}
