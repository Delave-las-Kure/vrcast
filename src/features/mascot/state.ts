/**
 * T327 — в каком настроении маскот, и почему именно в этом (FR-102).
 *
 * **Из тех же событий, что и экран задач.** Отдельный источник разошёлся бы с экраном, и
 * маскот радовался бы упавшей задаче — на экране красное, а он машет. Это не мелочь: он
 * заметнее любой строки, и человек читает сначала его.
 *
 * Правила здесь и только здесь, без React: их надо проверять без экрана, а событие «задача
 * упала» приходит в тот же миг, что «задача закончилась», и порядок между ними решает, что
 * человек увидит.
 */

import type {
  AppError,
  TaskDoneEvent,
  TaskProgressEvent,
  ViewersUpdateEvent,
} from "../../shared/contract";

/** Настроение. */
export type Mood = "idle" | "working" | "success" | "trouble" | "viewerTrouble";

/** Сколько держится настроение, наступившее от одного события, в миллисекундах. */
export const MOMENT_MS = 4000;

/** Что маскот знает о происходящем. */
export interface Mind {
  /** Сколько задач идёт прямо сейчас. */
  running: Set<string>;
  /** Настроение от последнего события и до какого момента оно держится. */
  moment: { mood: Mood; until: number } | null;
  /** Есть ли зритель, которому плохо. Держится, пока плохо, — это не миг, а состояние. */
  viewerInTrouble: boolean;
}

export function emptyMind(): Mind {
  return { running: new Set(), moment: null, viewerInTrouble: false };
}

/**
 * Что показывать сейчас.
 *
 * **Порядок разбора — это и есть правило.** Беда важнее успеха, а успех важнее работы:
 * маскот, показывающий «работаю» поверх только что упавшей задачи, прячет единственное, ради
 * чего на него смотрят. `now` передаётся, а не берётся из часов, — иначе проверить это можно
 * было бы только ожиданием.
 */
export function moodOf(mind: Mind, now: number): Mood {
  if (mind.moment && mind.moment.until > now) return mind.moment.mood;
  if (mind.viewerInTrouble) return "viewerTrouble";
  if (mind.running.size > 0) return "working";
  return "idle";
}

/** Задача подвинулась. */
export function onProgress(mind: Mind, e: TaskProgressEvent): Mind {
  const running = new Set(mind.running);
  if (e.state === "running") {
    running.add(e.id);
  } else {
    running.delete(e.id);
  }
  return { ...mind, running };
}

/** Задача закончилась — успехом, ошибкой или отменой. */
export function onDone(mind: Mind, e: TaskDoneEvent, now: number): Mind {
  const running = new Set(mind.running);
  running.delete(e.id);

  // Отмена — не беда и не успех: человек сам её и отменил, и хвалить его за это нелепо,
  // а тревожиться тем более.
  if (e.state === "cancelled") return { ...mind, running };

  const failed = isFailure(e);
  return {
    ...mind,
    running,
    moment: { mood: failed ? "trouble" : "success", until: now + MOMENT_MS },
  };
}

/** Список зрителей обновился. */
export function onViewers(mind: Mind, e: ViewersUpdateEvent): Mind {
  return { ...mind, viewerInTrouble: e.active.some((v) => v.problems.length > 0) };
}

/** Что говорит вспомогательным средствам. Настроение без слов — это картинка без смысла. */
export function moodLabel(mood: Mood, words: Record<string, string>): string {
  return words["mascot" + mood[0].toUpperCase() + mood.slice(1)] ?? "";
}

/** Была ли это ошибка. Отдельно, потому что `error` и `state` могут расходиться. */
export function isFailure(e: { state: string; error: AppError | null }): boolean {
  return e.state === "failed" || e.error !== null;
}
