/**
 * T471 — the stand-in has a method for every command, and says so when it does not.
 *
 * **Why this check and not trust.** A factory that goes stale fails in exactly the way the
 * hand-written objects it replaces did — a screen calls something it has not got, the call
 * throws inside an effect, and the render goes with it. Worse, it fails while *looking*
 * exhaustive, so nobody thinks to check it.
 *
 * It cannot go stale as written, because it is built from `Object.keys(ipc)`. This says that
 * out loud and would notice the day somebody replaces that with a list.
 */

import { describe, expect, it } from "vitest";

import { ipc } from "../ipc";
import { stubIpc } from "../../test-ipc";

describe("the stand-in for ipc", () => {
  it("has a method for every command the real one has", () => {
    const real = Object.keys(ipc).sort();
    const stub = Object.keys(stubIpc(ipc as unknown as Record<string, unknown>)).sort();
    expect(stub).toEqual(real);
    // And there is something to compare: an empty `ipc` would make the line above agree with
    // itself. The same emptiness that has caught the parsers in the core.
    expect(real.length).toBeGreaterThan(30);
  });

  it("answers a promise rather than nothing", async () => {
    // The fault this exists for. `undefined` is what a stub without an implementation returns
    // after `clearAllMocks`, and whatever called it then does `.then` on nothing.
    const stub = stubIpc(ipc as unknown as Record<string, unknown>);
    for (const name of Object.keys(stub)) {
      const answer = (stub[name] as () => unknown)();
      expect(answer, `${name} answered nothing`).toBeInstanceOf(Promise);
      await answer;
    }
  });

  it("lets a test replace only what it is about", async () => {
    const stub = stubIpc(ipc as unknown as Record<string, unknown>, {
      tasksList: () => Promise.resolve([{ id: "t1" }]),
    });
    await expect((stub.tasksList as () => Promise<unknown>)()).resolves.toEqual([{ id: "t1" }]);
    // Everything else still answers.
    await expect((stub.settingsGet as () => Promise<unknown>)()).resolves.toBeDefined();
  });

  it("refuses a name the real ipc has not got", () => {
    // A typo that quietly stubs nothing is a test checking something other than what it says.
    expect(() =>
      stubIpc(ipc as unknown as Record<string, unknown>, {
        tasksLits: () => Promise.resolve([]),
      }),
    ).toThrow(/tasksLits/);
  });
});
