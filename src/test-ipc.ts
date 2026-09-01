/**
 * T470 — a stand-in for `ipc` that answers every call.
 *
 * **The trap this closes, which caught three times in one day.** Every interface test writes
 * its own stub object by hand. A method a screen calls and the object does not have throws
 * *inside an effect* and takes the whole render with it — twenty-four tests failed at once
 * from one missing method when `Borrow` was added to the ladder screen. And a method the
 * object declares but never answers, after `clearAllMocks` has taken its implementation away,
 * returns `undefined`; whatever called it then does `.then` on nothing. Which caller and when
 * depends on when React gets round to an effect — sometimes one belonging to a test that has
 * already ended — so it passes on one machine and fails on another. It did: green here and
 * red in CI on 2026-08-28.
 *
 * So: one object with a method for every command, each answering something harmless, and a
 * test overrides only what it is about. Adding a call to a screen then breaks nothing.
 *
 * **Two hazards, both met head-on while this was written.**
 *
 * `vi.mock` is hoisted above every import in a file, so a name imported at the top is not
 * initialised when the factory runs: the suite then fails to load with `Cannot access
 * '__vi_import_3__' before initialization`, which says nothing about the cause. Import it
 * inside the factory instead.
 *
 * And it takes the real `ipc` as an argument rather than importing it. Importing it here
 * would mean this module reaching for `shared/ipc` from inside the factory that is *mocking*
 * `shared/ipc` — the factory calls itself, and the whole suite hangs with no message at all.
 * The test already holds the real one, from `vi.importActual`, so it hands it over:
 *
 * ```ts
 * vi.mock("../../../shared/ipc", async () => {
 *   const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
 *   const { stubIpc } = await import("../../../test-ipc");
 *   return { ...actual, ipc: stubIpc(actual.ipc, { tasksList: () => mockList() }) };
 * });
 * ```
 *
 * **What it deliberately does not do.** It answers, it does not pretend. A stub that returned
 * plausible-looking data would let a test pass while asserting on something the core never
 * said, which is a worse fault than the one this fixes.
 *
 * **One default for everything, and it is an empty array.** Not a table of a plausible shape
 * per command: that would be a second description of the contract kept by hand, and the day it
 * fell behind it would lie about shapes rather than about names — harder to notice and worse.
 * An empty array is the value that behaves as "nothing" in the most places: `.filter`, `.map`
 * and `.length` all work on it, spreading it adds nothing, and reading a field off it gives
 * `undefined` exactly as an empty object would. `{}` was tried first and a screen that
 * expected a list fell over on `.filter` — which is the same class of fault as the one this
 * module exists to remove, so it is written down rather than quietly corrected.
 */

/** Every command name the real `ipc` offers. */
export type IpcName = string;

/**
 * What a command answers when the test does not care what it answers.
 *
 * See the module's own note: one value for everything, and an array because it behaves as
 * nothing in the most shapes a caller can want.
 */
const NOTHING: readonly never[] = Object.freeze([]);

/**
 * A stand-in with every method the real `ipc` has.
 *
 * `over` replaces individual commands. Anything not named answers `NOTHING` — a resolved
 * promise, never `undefined`, which is the whole point.
 */
export function stubIpc(
  real: Record<string, unknown>,
  over: Record<string, unknown> = {},
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const name of Object.keys(real)) {
    const replacement = over[name];
    out[name] = replacement ?? (() => Promise.resolve(NOTHING));
  }
  // Named but not in `ipc` is a typo in a test, and a typo that silently does nothing is a
  // test that checks something other than what it says. Said out loud rather than dropped.
  for (const name of Object.keys(over)) {
    if (!(name in out)) {
      throw new Error(
        `stubIpc was given "${name}", which the real ipc has no such command for. ` +
          `Either the command was renamed and this test was not, or this is a typo — ` +
          `and a typo here would quietly stub nothing at all.`,
      );
    }
  }
  return out;
}
