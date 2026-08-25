/**
 * T102 — what becomes of the tasks if the application is closed (FR-086).
 *
 * The difference between kinds of task is not cosmetic, and a person has to learn
 * about it **before** closing rather than after. An upload continues from where it got
 * to and can be closed without a thought. Preparing a file is held by a live process
 * and will not survive: an hour of work would have to be repeated.
 *
 * The core says what happens to each one; this screen words it, in the language in use.
 */

import type { TaskOnClose } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";

export function CloseConsequences({ items }: { items: TaskOnClose[] }) {
  const t = useT();
  const { lang } = useLang();

  if (items.length === 0) return null;

  const losing = items.filter((task) => task.outcome === "restarts");

  return (
    <section
      className={`notice ${losing.length > 0 ? "notice--warning" : "notice--ok"}`}
      role="status"
    >
      <div className="notice__body">
        <strong className="notice__message">
          {losing.length > 0 ? t.ui.tasks.closeLosing : t.ui.tasks.closeSafe}
        </strong>
        <ul className="notice__list">
          {items.map((task) => (
            <li key={task.id} className={`consequence consequence--${task.outcome}`}>
              {renderDetail(task.explanation, t, lang)}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
