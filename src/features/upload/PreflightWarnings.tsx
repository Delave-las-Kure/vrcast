/**
 * T099 — the warnings the core raises before an upload starts.
 *
 * The wording comes from the shared catalogue by the codes the core sent (FR-105):
 * the screen does not invent phrases of its own, or the same trouble would be
 * explained differently on different screens.
 *
 * Only one thing is decided here, and it matters: whether to show this as a question
 * or as a refusal. The difference is not cosmetic. A shortage of room cannot be waved
 * away by agreeing — agreement does not create space, and an "upload anyway" button
 * would be a lie: the transfer would hit the end of the disk halfway through thirty
 * gigabytes. A taken name and someone watching, on the other hand, are exactly a
 * question: the person may well know what they are doing.
 */

import type { AppError } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import { renderError } from "../../shared/i18n/render";

/** Refusals that a person's agreement lifts. */
const LIFTED_BY_AGREEMENT = ["NAME_EXISTS", "VIEWERS_ACTIVE", "CONFIRMATION_REQUIRED"];

export function canConfirm(error: AppError): boolean {
  return LIFTED_BY_AGREEMENT.includes(error.code);
}

export function PreflightWarnings({
  error,
  onConfirm,
  onCancel,
  busy,
}: {
  error: AppError;
  onConfirm: () => void;
  onCancel: () => void;
  busy: boolean;
}) {
  const t = useT();
  const { lang } = useLang();
  const liftable = canConfirm(error);
  const { message, hint } = renderError(error, t, lang);

  return (
    <div
      className={`notice ${liftable ? "notice--warning" : "notice--error"}`}
      role={liftable ? "status" : "alert"}
    >
      <div className="notice__body">
        <strong className="notice__message">{message}</strong>
        {hint && <p className="notice__hint">{hint}</p>}
        {error.cause && !liftable && <p className="notice__cause">{error.cause}</p>}

        <div className="notice__actions">
          {liftable ? (
            <>
              <button onClick={onConfirm} disabled={busy}>
                {t.ui.preflight.uploadAnyway}
              </button>
              <button className="button--quiet" onClick={onCancel} disabled={busy}>
                {t.ui.common.cancel}
              </button>
            </>
          ) : (
            <button className="button--quiet" onClick={onCancel}>
              {t.ui.preflight.understood}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
