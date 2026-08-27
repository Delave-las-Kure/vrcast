/**
 * T322, T327, T328 — маскот, его настроения и его выключатель (FR-102, FR-103).
 *
 * **Выключённый маскот не загружается вовсе.** Иначе «выключён» означает лишь «не виден», а
 * ресурсы он тратит по-прежнему — и ровно на слабой машине, где его и выключили. Рисунок
 * лежит отдельным файлом и подтягивается по требованию: при выключенной настройке ветка с
 * `lazy` не выполняется, и запроса за ним не происходит. Проверяется это **отсутствием
 * запроса**, а не отсутствием картинки (T330): картинки нет и когда она просто не
 * отрисовалась.
 *
 * **Настроение берётся из тех же событий, что и экран задач.** Правила — в `state.ts`, здесь
 * только подписка: разделено, чтобы правила можно было проверить без экрана.
 */

import { Suspense, lazy, useEffect, useMemo, useRef, useState } from "react";

import {
  emptyMind,
  moodLabel,
  moodOf,
  onDone,
  onProgress,
  onViewers,
  type Mind,
  type Mood,
} from "./state";
import { useSettings } from "../../app/settings";
import { useT } from "../../shared/i18n";
import { onTaskDone, onTaskProgress, onViewersUpdate } from "../../shared/ipc";

/**
 * Загружается по требованию, и это здесь главное.
 *
 * Вынесено на уровень модуля, потому что `lazy` внутри тела компонента заводил бы новый
 * ленивый модуль на каждую перерисовку — и рисунок запрашивался бы снова и снова.
 */
const Drawing = lazy(() => import("./MascotDrawing"));

/** Как часто пересчитывать настроение: миг от события истекает сам собой. */
const TICK_MS = 500;

export function Mascot() {
  const t = useT();
  const { settings, error } = useSettings();
  const mind = useRef<Mind>(emptyMind());
  const [mood, setMood] = useState<Mood>("idle");

  /**
   * Пока не известно — не показываем и, главное, **не грузим**.
   *
   * Соблазн был обратный: маскот включён по умолчанию, так что «пока не прочитали — считаем
   * включённым» выглядело разумно. Оно и работало — рисунок уезжал в загрузку на первой же
   * отрисовке, до того как ядро успевало ответить «выключен», и настройка не мешала ему
   * ровно ничем. Поймано проверкой `mascot-off`, которая для этого и написана.
   *
   * Если ядро не ответило вовсе (`error`), берётся значение по умолчанию: не показать
   * маскота из-за недоступной базы — это наказать человека за чужую поломку.
   */
  const known = settings !== null || error !== null;
  const shown = known && settings?.mascot !== false;

  useEffect(() => {
    if (!shown) return;
    let alive = true;
    const refresh = () => {
      if (alive) setMood(moodOf(mind.current, Date.now()));
    };

    const stops: Promise<() => void>[] = [
      onTaskProgress((e) => {
        mind.current = onProgress(mind.current, e);
        refresh();
      }),
      onTaskDone((e) => {
        mind.current = onDone(mind.current, e, Date.now());
        refresh();
      }),
      onViewersUpdate((e) => {
        mind.current = onViewers(mind.current, e);
        refresh();
      }),
    ];

    // Миг от события истекает без всякого события, и без этого маскот застревал бы в
    // «получилось» до следующей задачи — то есть иногда навсегда.
    const timer = setInterval(refresh, TICK_MS);

    return () => {
      alive = false;
      clearInterval(timer);
      for (const stop of stops) void stop.then((f) => f());
    };
  }, [shown]);

  const words = useMemo(() => t.ui.appearance as unknown as Record<string, string>, [t]);

  if (!shown) return null;

  return (
    <div className="mascot-slot" data-testid="mascot-slot">
      {/* Пусто, пока рисунок едет. Заглушки нет намеренно: мигнуть серым кружком и
          заменить его маскотом — это два движения там, где хватит одного. */}
      <Suspense fallback={null}>
        <Drawing mood={mood} label={moodLabel(mood, words)} />
      </Suspense>
    </div>
  );
}
