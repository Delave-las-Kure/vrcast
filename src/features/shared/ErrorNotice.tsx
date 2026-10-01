/**
 * Showing an error.
 *
 * The core names the situation and the values that go with it; the wording is looked
 * up here, in the catalogue of the chosen language (FR-105, FR-106). One catalogue
 * means one wording per situation, so the same trouble is never explained two ways on
 * two screens — which is what the rule was written for.
 *
 * T674: one line in sight, the rest under "Details", folded by default — what to do, the
 * particulars the core named, and the technical cause, in that order. Nothing is dropped:
 * the advice and the particulars are one click away, not gone.
 */

import type { AppError } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import { renderErrorFolded } from "../../shared/i18n/render";

/**
 * The line and the folded rest, without a frame of its own.
 *
 * Shared by the banner below, a failed task's row and a refusal inside a dialog, so the
 * three cannot drift about what a code means or what is folded away.
 */
export function ErrorFolded({ error, lineClassName }: { error: AppError; lineClassName: string }) {
  const t = useT();
  const { lang } = useLang();
  const { line, hint, particulars } = renderErrorFolded(error, t, lang);
  const more = Boolean(hint || particulars || error.cause);

  return (
    <>
      <p className={lineClassName}>{line}</p>
      {more && (
        <details className="error-more">
          <summary>{t.ui.common.more}</summary>
          {hint && <p className="error-more__hint">{hint}</p>}
          {particulars && <p className="error-more__particulars">{particulars}</p>}
          {error.cause && <p className="error-more__cause">{error.cause}</p>}
        </details>
      )}
    </>
  );
}

export function ErrorNotice({ error, onDismiss }: { error: AppError; onDismiss?: () => void }) {
  const t = useT();

  return (
    <div className="notice notice--error" role="alert">
      <div className="notice__body">
        <ErrorFolded error={error} lineClassName="notice__message" />
      </div>
      {onDismiss && (
        <button className="notice__close" onClick={onDismiss} aria-label={t.ui.common.dismiss}>
          ×
        </button>
      )}
    </div>
  );
}
