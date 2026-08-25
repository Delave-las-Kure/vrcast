/**
 * T099 — предупреждения, которые ядро выдаёт до начала заливки.
 *
 * Формулировки берутся у ядра как есть (FR-105): интерфейс их не сочиняет и не
 * переписывает, иначе одна и та же беда объяснялась бы по-разному на разных экранах.
 *
 * Здесь решается только одно, но важное: показать это как вопрос или как отказ.
 * Разница не косметическая. Нехватку места подтверждением не снять — места
 * от согласия не появится, и кнопка «всё равно залить» была бы обманом: передача
 * упрётся в конец диска на середине тридцати гигабайт. А занятое имя и идущий
 * просмотр — именно вопрос: человек может знать, что делает.
 */

import type { AppError } from "../../shared/contract";

/** Отказы, которые снимаются согласием человека. */
const СНИМАЕТСЯ_СОГЛАСИЕМ = ["NAME_EXISTS", "VIEWERS_ACTIVE", "CONFIRMATION_REQUIRED"];

export function canConfirm(error: AppError): boolean {
  return СНИМАЕТСЯ_СОГЛАСИЕМ.includes(error.code);
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
  const снимается = canConfirm(error);

  return (
    <div
      className={`notice ${снимается ? "notice--warning" : "notice--error"}`}
      role={снимается ? "status" : "alert"}
    >
      <div className="notice__body">
        <strong className="notice__message">{error.message}</strong>
        {error.hint && <p className="notice__hint">{error.hint}</p>}
        {error.cause && !снимается && <p className="notice__cause">{error.cause}</p>}

        <div className="notice__actions">
          {снимается ? (
            <>
              <button onClick={onConfirm} disabled={busy}>
                Всё равно залить
              </button>
              <button className="button--quiet" onClick={onCancel} disabled={busy}>
                Отмена
              </button>
            </>
          ) : (
            <button className="button--quiet" onClick={onCancel}>
              Понятно
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
