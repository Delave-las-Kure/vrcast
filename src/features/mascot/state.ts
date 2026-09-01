/**
 * T327 — what mood the mascot is in, and why that one (FR-102).
 *
 * **From the same events as the task screen.** A source of its own would drift from the
 * screen, and the mascot would be cheerful about a task that failed — red on the list and
 * waving beside it. That is not a small thing: it catches the eye before any row does.
 *
 * The rules live here and nowhere else, without React: they have to be checked without a
 * screen, and "the task failed" arrives in the same instant as "the task ended" — the order
 * between them decides what a person sees.
 */

import type {
  AppError,
  TaskDoneEvent,
  TaskProgressEvent,
  ViewersUpdateEvent,
} from "../../shared/contract";

/** A mood. */
export type Mood = "idle" | "working" | "success" | "trouble" | "viewerTrouble";

/** How long a mood brought on by one event lasts, in milliseconds. */
export const MOMENT_MS = 4000;

/** What the mascot knows about what is going on. */
export interface Mind {
  /** How many tasks are running right now. */
  running: Set<string>;
  /** The mood the last event brought on, and the moment it lapses. */
  moment: { mood: Mood; until: number } | null;
  /** Whether a viewer is having a bad time. It lasts while they are: a state, not an instant. */
  viewerInTrouble: boolean;
}

export function emptyMind(): Mind {
  return { running: new Set(), moment: null, viewerInTrouble: false };
}

/**
 * What to show now.
 *
 * **The order of the branches is the rule.** Trouble outranks success and success outranks
 * work: a mascot showing "busy" over a task that has just failed hides the one thing
 * anybody looks at it for. `now` is handed in rather than read off a clock, or checking this
 * would mean waiting for it.
 */
export function moodOf(mind: Mind, now: number): Mood {
  if (mind.moment && mind.moment.until > now) return mind.moment.mood;
  if (mind.viewerInTrouble) return "viewerTrouble";
  if (mind.running.size > 0) return "working";
  return "idle";
}

/** A task moved. */
export function onProgress(mind: Mind, e: TaskProgressEvent): Mind {
  const running = new Set(mind.running);
  if (e.state === "running") {
    running.add(e.id);
  } else {
    running.delete(e.id);
  }
  return { ...mind, running };
}

/** A task ended — in success, in failure, or cancelled. */
export function onDone(mind: Mind, e: TaskDoneEvent, now: number): Mind {
  const running = new Set(mind.running);
  running.delete(e.id);

  // Cancelling is neither trouble nor success: the person did it themselves, and praising
  // them for it would be odd, while worrying about it would be odder.
  if (e.state === "cancelled") return { ...mind, running };

  const failed = isFailure(e);
  return {
    ...mind,
    running,
    moment: { mood: failed ? "trouble" : "success", until: now + MOMENT_MS },
  };
}

/** The list of viewers came round again. */
export function onViewers(mind: Mind, e: ViewersUpdateEvent): Mind {
  return { ...mind, viewerInTrouble: e.active.some((v) => v.problems.length > 0) };
}

/** What it tells assistive software. A mood with no words is a picture with no meaning. */
export function moodLabel(mood: Mood, words: Record<string, string>): string {
  return words["mascot" + mood[0].toUpperCase() + mood.slice(1)] ?? "";
}

/** Whether this was a failure. Separate, because `error` and `state` can disagree. */
export function isFailure(e: { state: string; error: AppError | null }): boolean {
  return e.state === "failed" || e.error !== null;
}
