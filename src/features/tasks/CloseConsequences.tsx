/**
 * T102 — что станет с задачами, если закрыть приложение (FR-086).
 *
 * Разница между видами задач не косметическая, и человек обязан узнать о ней **до**
 * закрытия, а не после. Заливка продолжится с достигнутого места — её можно закрывать
 * спокойно. Подготовка файла держится живым процессом и закрытия не переживёт:
 * час работы придётся повторить.
 *
 * Формулировки приходят от ядра готовыми: оно одно знает, сколько именно потеряется.
 */

import type { TaskOnClose } from "../../shared/contract";

export function CloseConsequences({ items }: { items: TaskOnClose[] }) {
  if (items.length === 0) return null;

  const теряется = items.filter((t) => t.outcome === "restarts");

  return (
    <section
      className={`notice ${теряется.length > 0 ? "notice--warning" : "notice--ok"}`}
      role="status"
    >
      <div className="notice__body">
        <strong className="notice__message">
          {теряется.length > 0
            ? "Если закрыть приложение сейчас, часть работы потеряется"
            : "Приложение можно закрыть: незаконченное продолжится при запуске"}
        </strong>
        <ul className="notice__list">
          {items.map((t) => (
            <li key={t.id} className={`consequence consequence--${t.outcome}`}>
              {t.explanation}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
