/**
 * T101 — перестановка очереди (FR-083).
 *
 * Кнопки «выше»/«ниже», а не перетаскивание: перетаскивание требует мыши, точности
 * и не работает с клавиатуры, а очередь чаще всего правят на одну позицию — «вот это
 * нужно раньше». Список тут короткий, и двух кнопок достаточно.
 *
 * Переставляются только ждущие задачи. Выполняющаяся в этом списке не появляется
 * вовсе: прервать начатую передачу ради изменения порядка значило бы выбросить часы
 * работы, и показывать для неё кнопку было бы обещанием того, чего не будет.
 */

import type { Task } from "../../shared/contract";

const KIND_LABEL: Record<string, string> = {
  probe: "разбор исходника",
  convert: "подготовка файла",
  upload: "заливка на сервер",
  build_ladder: "сборка набора качеств",
  deploy: "развёртывание",
  upgrade_server: "обновление сервера",
  diagnose: "диагностика",
};

export function QueueOrder({
  queued,
  onReorder,
  busy,
}: {
  /** Ждущие задачи в том порядке, в каком они пойдут в работу. */
  queued: Task[];
  onReorder: (orderedIds: string[]) => void;
  busy: boolean;
}) {
  if (queued.length === 0) return null;

  const переставить = (from: number, to: number) => {
    if (to < 0 || to >= queued.length) return;
    const ids = queued.map((t) => t.id);
    const [взятая] = ids.splice(from, 1);
    ids.splice(to, 0, взятая);
    onReorder(ids);
  };

  return (
    <section className="queue">
      <h2>В очереди</h2>
      <p className="muted">
        Задачи пойдут в этом порядке. Уже начатую перестановка не трогает — её
        пришлось бы прервать, потеряв сделанное.
      </p>
      <ol className="queue__list">
        {queued.map((t, i) => (
          <li key={t.id} className="queue__item">
            <span className="queue__position">{i + 1}</span>
            <span className="queue__kind">{KIND_LABEL[t.kind] ?? t.kind}</span>
            <span className="queue__actions">
              <button
                aria-label="Поднять в очереди"
                title="Поднять в очереди"
                disabled={busy || i === 0}
                onClick={() => переставить(i, i - 1)}
              >
                ↑
              </button>
              <button
                aria-label="Опустить в очереди"
                title="Опустить в очереди"
                disabled={busy || i === queued.length - 1}
                onClick={() => переставить(i, i + 1)}
              >
                ↓
              </button>
            </span>
          </li>
        ))}
      </ol>
    </section>
  );
}
