/**
 * T058 — the mark that says this is the last known state.
 *
 * It appears when the server is out of reach. Both alternatives are worse: a blank
 * screen is indistinguishable from "the library is gone", and an endless spinner from
 * "the application has hung". A person needs exactly two things: the data is real but
 * old, and there is no connection to the server right now.
 */

import { useT } from "../../shared/i18n";

export function StaleBanner({ onRetry }: { onRetry?: () => void }) {
  const t = useT();

  return (
    <div className="notice notice--stale" role="status">
      <div className="notice__body">
        <strong className="notice__message">{t.ui.library.staleTitle}</strong>
        <p className="notice__hint">{t.ui.library.staleHint}</p>
      </div>
      {onRetry && <button onClick={onRetry}>{t.ui.library.staleRetry}</button>}
    </div>
  );
}
