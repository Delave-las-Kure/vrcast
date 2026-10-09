/**
 * T058 — the mark that says this is the last known state.
 *
 * It appears when the server is out of reach. Both alternatives are worse: a blank
 * screen is indistinguishable from "the library is gone", and an endless spinner from
 * "the application has hung". A person needs exactly two things: the data is real but
 * old, and there is no connection to the server right now.
 *
 * T702 — one line, with the time the list is from, and «Retry» beside it: «The server is not
 * answering — shown as of 14:05». The time is that of the last read the server answered.
 */

import { useLang, useT } from "../../shared/i18n";
import type { Lang } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

/** «14:05» today; «08.10 14:05» on another day. In the computer's own time. */
export function shownAt(readAt: string, lang: Lang, now: Date = new Date()): string | null {
  const at = new Date(readAt);
  if (Number.isNaN(at.getTime())) return null;
  const locale = lang === "ru" ? "ru-RU" : "en-GB";
  const time = at.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
  if (at.toDateString() === now.toDateString()) return time;
  const day = at.toLocaleDateString(locale, { day: "2-digit", month: "2-digit" });
  return `${day} ${time}`;
}

export function StaleBanner({ readAt, onRetry }: { readAt?: string | null; onRetry?: () => void }) {
  const t = useT();
  const { lang } = useLang();
  const l = t.ui.library;
  const time = readAt ? shownAt(readAt, lang) : null;

  return (
    <div className="notice notice--stale" role="status" data-testid="stale">
      <div className="notice__body">
        <strong className="notice__message">
          {time ? fill(l.staleAt, { time }, t, lang) : l.staleTitle}
        </strong>
      </div>
      {onRetry && <button onClick={onRetry}>{l.staleRetry}</button>}
    </div>
  );
}
