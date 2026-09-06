/**
 * An error has two halves, and showing one of them is the easy mistake.
 *
 * ⚠ **T519, found 2026-09-06.** `renderError` hands back `{ message, hint }`, and its own
 * comment says what the second one is for: the message is what happened, the hint is what to
 * do about it, and the hint comes from the code rather than from the particulars precisely so
 * that it is always the same advice for the same trouble. Three places took `.message` and
 * dropped `.hint` — including the task list, which is the one place a person meets a failed
 * task at all. So the advice existed, was written in both languages, was tested, and reached
 * nobody.
 *
 * **Why a scan and not three tests.** A test per screen is what was already there: every one
 * of the three had tests, and every one of them passed, because a test asserts what the
 * screen shows and none of them thought to assert what it does not. The question "does any
 * screen throw half the answer away" cannot be asked one screen at a time — it has to be
 * asked of all of them at once, which is what this does.
 *
 * **The sources come from the bundler**, for the reason `reachable.test.ts` gives: reading
 * through `import.meta.glob` keeps this file inside the browser-side `tsconfig`, where
 * `node:fs` would have meant teaching the application's type checking about Node.
 */

import { expect, it } from "vitest";

/**
 * Places that legitimately want the message alone, each with the reason.
 *
 * **Two kinds of thing are not a failure**, and both are here. A refusal that carries numbers
 * for a confirmation is a question, not a fault: its "what to do" is the dialog's own two
 * buttons. And a system notification is one line by construction — the advice waits in the
 * task list, which is where the notification is sending the person anyway.
 */
const MESSAGE_IS_THE_WHOLE_ANSWER: Array<[string, string]> = [
  [
    "features/library/LibraryScreen.tsx",
    "Both call sites turn a CONFIRMATION_REQUIRED refusal into the `consequences` line of a " +
      "delete dialog. That refusal is not a failure — it is the core declining to act until " +
      "asked twice, and it carries the file count and the volume so the dialog can say what " +
      "will go. What to do about it is the dialog's own two buttons, and a hint reading like " +
      "advice about a fault would misdescribe a question as a problem.",
  ],
  [
    "features/tasks/notifications.tsx",
    "The body of a system notification, which the platform gives two lines and truncates. It " +
      "exists to say a long task ended badly while the window was out of sight; the advice " +
      "is in the task list, where the notification is already sending the person. Adding the " +
      "hint here would push the message itself out of view.",
  ],
];

const RAW = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** The path as the excuse list writes it: relative to `src/`, forward slashes. */
function tidy(path: string): string {
  return path.replace(/^\.\.\//, "");
}

/** Everything but this project's own tests and the renderer being checked. */
function screens(): Array<[string, string]> {
  return Object.entries(RAW)
    .map(([path, text]) => [tidy(path), text] as [string, string])
    .filter(([path]) => !path.includes("__tests__"))
    .filter(([path]) => path !== "shared/i18n/render.ts");
}

it("no screen shows what happened and throws away what to do about it", () => {
  const excused = new Set(MESSAGE_IS_THE_WHOLE_ANSWER.map(([path]) => path));
  const guilty: string[] = [];

  for (const [path, text] of screens()) {
    // `renderError(...).message` — the whole answer fetched and half of it dropped in one
    // expression. Destructuring is the other shape, and it is checked below.
    if (/renderError\([^)]*\)\s*\.message\b/.test(text) && !excused.has(path)) {
      guilty.push(`${path}: renderError(...).message`);
    }
    // `const { message } = renderError(...)` — the hint not even named.
    const destructured = text.match(/const\s*\{([^}]*)\}\s*=\s*renderError\(/g) ?? [];
    for (const one of destructured) {
      if (!one.includes("hint") && !excused.has(path)) {
        guilty.push(`${path}: ${one.trim()}`);
      }
    }
  }

  expect(
    guilty,
    "these take an error's message and drop its hint, which is the half that says what to " +
      "do. Show both, or put the file in MESSAGE_IS_THE_WHOLE_ANSWER with the reason the " +
      "message alone is the whole answer there.",
  ).toEqual([]);
});

it("every excuse still names a file that renders an error", () => {
  const known = new Map(screens());
  const rotten = MESSAGE_IS_THE_WHOLE_ANSWER.filter(([path]) => {
    const text = known.get(path);
    return text === undefined || !text.includes("renderError");
  }).map(([path]) => path);

  expect(
    rotten,
    "these excuses name a file that is gone or no longer renders an error at all. Delete " +
      "the entry — what it described is not there any more.",
  ).toEqual([]);
});

/**
 * The scan must reach something, or both checks above pass over an empty set.
 *
 * The failure this rules out has no symptom of its own: a glob that stopped matching, or a
 * filter that swallowed everything, leaves two green tests that check nothing — which is
 * worse than no check, because it is believed.
 */
it("the scan reaches the screens it is about", () => {
  const found = screens();
  expect(found.length).toBeGreaterThan(20);

  const renderers = found.filter(([, text]) => text.includes("renderError("));
  expect(
    renderers.map(([path]) => path).sort(),
    "the files that render an error are not the ones expected; if a screen was added or " +
      "removed this list is what says so, and it must be updated deliberately",
  ).toEqual([
    "features/library/LibraryScreen.tsx",
    "features/library/dialogs/MediaDialogs.tsx",
    "features/shared/ErrorNotice.tsx",
    "features/tasks/TasksPanel.tsx",
    "features/tasks/notifications.tsx",
    "features/upload/PreflightWarnings.tsx",
  ]);
});
