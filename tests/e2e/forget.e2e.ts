/**
 * T359 — "remove my data" pressed in a real window, against a real directory (FR-114).
 *
 * **Why this exists when there are already unit tests for it.** The fault this whole group of
 * tasks came from (T356) was not a wrong function: it was a correct function pointed at the
 * wrong directory. The uninstaller's checkbox removed `%APPDATA%\\ru.vrcast.studio` while the
 * application kept everything in `%APPDATA%\\VRCast\\VRCast Studio`. Every test of the removal
 * itself passed, because they all agreed with each other about where the data was. Only the
 * built application knows where it actually writes, and only pressing the real button proves
 * that the path it names is the path it uses.
 *
 * **Linux only, and that is a limit, not a preference.** The data directory has to be moved
 * somewhere disposable before anything is removed, and on Linux it can be: `directories` reads
 * `XDG_DATA_HOME`. On Windows it asks the system for the known folder and ignores the
 * environment entirely, so the same test there would delete the developer's own profiles — the
 * accident that has already happened once on this machine, and not one to arrange deliberately.
 * The Windows side is covered where it can be: the uninstaller hook's truth table
 * (`src-tauri/tests/uninstall-hook/`) and scenario 10 of the quickstart, walked by hand.
 *
 * **The safety catch is load-bearing.** Before anything is pressed, the directory the
 * application names must be inside this test's scratch. If the redirection did not take, the
 * application is pointed at the person's real data, and the test stops rather than proving its
 * point on somebody's profiles.
 *
 * **What is deliberately not checked here: the secrets.** They live in the operating system's
 * own store, and in the container these tests run in there is no such store to put one in. A
 * removal with no secrets in it cannot say anything about removing secrets — so it does not
 * pretend to. That half is checked in `src-tauri/tests/contract/forget.rs`, against a store
 * made for the test.
 */

import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Harness } from "./session";
import { ensureDriver } from "../../scripts/fetch-webdriver.mjs";

const onLinux = process.platform === "linux";

// Said out loud rather than skipped in silence: a file that quietly does nothing on the
// platform somebody is standing on reads exactly like a file that passed.
if (!onLinux) {
  console.warn(
    `T359 does not run on ${process.platform}: the data directory cannot be pointed somewhere ` +
      "disposable there, and the test would remove the real one. The Windows side is covered " +
      "by the uninstaller hook's truth table and by scenario 10 of the quickstart.",
  );
}

let scratch: string;
let harness: Harness | undefined;
/** The directory the application itself named on screen. Filled in by the first test. */
let named: string | undefined;

describe.skipIf(!onLinux)("removing everything, from the window", () => {
  beforeAll(async () => {
    scratch = mkdtempSync(join(tmpdir(), "vrcast-forget-"));
    const nativeDriver = await ensureDriver();
    harness = await Harness.start(nativeDriver, {
      XDG_DATA_HOME: join(scratch, "data"),
      XDG_CONFIG_HOME: join(scratch, "config"),
      XDG_CACHE_HOME: join(scratch, "cache"),
    });
  }, 120_000);

  afterAll(async () => {
    await harness?.stop();
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  });

  it("names the directory it actually writes to, and it is ours", async () => {
    if (!harness) throw new Error("the application did not start");

    const link = await harness.session.find('a[href="#/appearance"]');
    await link.click();

    const list = await harness.session.findFilled('[data-testid="forget-list"]');
    const said = await list.text();

    // The directory named on screen has to be the one under this test's scratch. If it is not,
    // the redirection did not take and the next test would be pressing "remove" on the real
    // one — so this failure has to stop the run, not annoy it.
    expect(said).toContain(scratch);

    // Pulled out of the screen rather than worked out here: the point of this test is that the
    // path the application *names* is the path it *writes to*, and guessing the second one
    // would be assuming exactly what is in question (T356).
    named = /(\/[^\s]*vrcast-forget-[^\s]*)/.exec(said)?.[1];
    expect(named).toBeDefined();

    // And it has to be a directory that exists, with something in it: a name on a screen is
    // not a directory.
    expect(existsSync(named as string)).toBe(true);
    expect(readdirSync(named as string).length).toBeGreaterThan(0);
  }, 60_000);

  it("removes it when the button is pressed, and says so", async () => {
    if (!harness) throw new Error("the application did not start");

    // The catch again, in the test that does the pressing: this one is not allowed to run on
    // anything but our own scratch, whatever the test before it concluded.
    if (!named || !named.includes(scratch)) {
      throw new Error("the application did not name a directory inside our scratch — not pressing");
    }
    if (!existsSync(named)) throw new Error("the named directory is not there — not pressing");
    expect(readdirSync(named).length).toBeGreaterThan(0);

    const agree = await harness.session.find('[data-testid="forget-agree"]');
    await agree.click();
    const button = await harness.session.find('[data-testid="forget-do"]');
    await button.click();

    // "Done" appears when the removal has actually happened, not when it was asked for.
    const done = await harness.session.findFilled('[data-testid="forget-done"]');
    expect((await done.text()).trim().length).toBeGreaterThan(0);

    // And it must not be a "done" with a complaint attached: that message means the directory
    // is still there, and the person has been told the opposite.
    expect(await harness.session.has('[data-testid="forget-dir-left"]')).toBe(false);

    // The directory the application named is the directory that has to be gone. Checking a
    // parent instead would pass on a run where something else happened to leave it empty.
    expect(existsSync(named)).toBe(false);
  }, 60_000);
});
