/**
 * Showing an error.
 *
 * The core names the situation and the values that go with it; the wording is looked
 * up here, in the catalogue of the chosen language (FR-105, FR-106). One catalogue
 * means one wording per situation, so the same trouble is never explained two ways on
 * two screens — which is what the rule was written for.
 */

import type { AppError } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import { renderError } from "../../shared/i18n/render";

export function ErrorNotice({
  error,
  onDismiss,
}: {
  error: AppError;
  onDismiss?: () => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const { message, hint } = renderError(error, t, lang);

  return (
    <div className="notice notice--error" role="alert">
      <div className="notice__body">
        <strong className="notice__message">{message}</strong>
        {hint && <p className="notice__hint">{hint}</p>}
        {error.cause && <p className="notice__cause">{error.cause}</p>}
      </div>
      {onDismiss && (
        <button
          className="notice__close"
          onClick={onDismiss}
          aria-label={t.ui.common.dismiss}
        >
          ×
        </button>
      )}
    </div>
  );
}
