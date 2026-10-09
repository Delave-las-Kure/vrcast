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

  // T710 — one line per thing said. Two tasks of the same kind used to give the same sentence
  // twice, word for word, and a list that repeats itself reads as a glitch rather than as two
  // things at stake. Said once, with how many it is about.
  const lines: { text: string; outcome: string; n: number; key: string }[] = [];
  for (const task of items) {
    const text = renderDetail(task.explanation, t, lang);
    const same = lines.find((l) => l.text === text);
    if (same) same.n += 1;
    else lines.push({ text, outcome: task.outcome, n: 1, key: task.id });
  }

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
          {lines.map((line) => (
            <li key={line.key} className={`consequence consequence--${line.outcome}`}>
              {line.n > 1 ? `${line.text} (×${line.n})` : line.text}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
