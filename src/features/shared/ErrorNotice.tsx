/**
 * Показ ошибки.
 *
 * Сообщение и подсказка приходят от ядра уже готовыми (FR-105) — интерфейс их не сочиняет
 * и не переписывает. Иначе формулировки разойдутся между экранами, и одна и та же беда
 * будет объясняться по-разному.
 */

import type { AppError } from "../../shared/contract";

export function ErrorNotice({
  error,
  onDismiss,
}: {
  error: AppError;
  onDismiss?: () => void;
}) {
  return (
    <div className="notice notice--error" role="alert">
      <div className="notice__body">
        <strong className="notice__message">{error.message}</strong>
        <p className="notice__hint">{error.hint}</p>
        {error.cause && <p className="notice__cause">{error.cause}</p>}
      </div>
      {onDismiss && (
        <button className="notice__close" onClick={onDismiss} aria-label="Скрыть">
          ×
        </button>
      )}
    </div>
  );
}
