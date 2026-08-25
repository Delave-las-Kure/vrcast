/**
 * T101 — reordering the queue (FR-083).
 *
 * Up/down buttons rather than dragging: dragging needs a mouse and precision and does
 * not work from the keyboard, while a queue is most often corrected by one position —
 * "this one should go first". The list is short, and two buttons are enough.
 *
 * Only waiting tasks are reordered. A running one does not appear in this list at all:
 * interrupting a transfer under way to change the order would throw away hours of
 * work, and showing a button for it would be promising something that will not happen.
 */

import type { Task, TaskKind } from "../../shared/contract";
import { useT } from "../../shared/i18n";

export function QueueOrder({
  queued,
  onReorder,
  busy,
}: {
  /** Waiting tasks, in the order they will run. */
  queued: Task[];
  onReorder: (orderedIds: string[]) => void;
  busy: boolean;
}) {
  const t = useT();

  if (queued.length === 0) return null;

  const move = (from: number, to: number) => {
    if (to < 0 || to >= queued.length) return;
    const ids = queued.map((task) => task.id);
    const [taken] = ids.splice(from, 1);
    ids.splice(to, 0, taken);
    onReorder(ids);
  };

  const kindLabel = (kind: TaskKind) => t.ui.tasks.kinds[kind] ?? kind;

  return (
    <section className="queue">
      <h2>{t.ui.tasks.queueHeading}</h2>
      <p className="muted">{t.ui.tasks.queueExplain}</p>
      <ol className="queue__list">
        {queued.map((task, i) => (
          <li key={task.id} className="queue__item">
            <span className="queue__position">{i + 1}</span>
            <span className="queue__kind">{kindLabel(task.kind)}</span>
            <span className="queue__actions">
              <button
                aria-label={t.ui.tasks.moveUp}
                title={t.ui.tasks.moveUp}
                disabled={busy || i === 0}
                onClick={() => move(i, i - 1)}
              >
                ↑
              </button>
              <button
                aria-label={t.ui.tasks.moveDown}
                title={t.ui.tasks.moveDown}
                disabled={busy || i === queued.length - 1}
                onClick={() => move(i, i + 1)}
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
