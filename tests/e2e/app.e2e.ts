/**
 * T342 — the application, driven as a person drives it.
 *
 * This is the level nothing else covers. The unit checks know the screens render from a
 * catalogue; the contract checks know the core answers what it promised. Neither of them
 * knows that the built binary **opens at all** — that the frontend really got embedded, that
 * the webview starts, that the sections are reachable. Every one of those has broken in
 * other projects between a green test suite and a shipped build.
 *
 * Kept deliberately thin. An end-to-end check is the slowest and most brittle kind there is,
 * and one that re-tests what the fast checks already cover buys nothing and breaks often.
 * What is here is what only this level can answer.
 */

import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Harness } from "./session";
import { ensureDriver } from "../../scripts/fetch-webdriver.mjs";

let harness: Harness;

beforeAll(async () => {
  const nativeDriver = await ensureDriver();
  harness = await Harness.start(nativeDriver);
}, 120_000);

afterAll(async () => {
  await harness?.stop();
});

describe("the built application", () => {
  it("opens and shows its sections", async () => {
    // The frontend is embedded into the binary at build time. When that goes wrong the window
    // opens white and every other check in the project still passes.
    const nav = await harness.session.find("nav.sidebar");
    const text = await nav.text();
    expect(text.length).toBeGreaterThan(0);
    // The brand, which is the one string that is not translated and not read from the core.
    expect(await (await harness.session.find(".sidebar__title")).text()).toContain("VRCast");
  }, 60_000);

  it("opens on the task section rather than on nothing", async () => {
    // A shell that starts on a blank route looks broken to somebody who has just installed
    // it, and "it opened but was empty" is what they will report.
    expect(await harness.session.has(".content")).toBe(true);
    // Waiting, not sampling: the section fills from the core, and on a machine starting for
    // the first time that takes a moment the sample lands inside.
    const content = await harness.session.findFilled(".content");
    expect((await content.text()).trim().length).toBeGreaterThan(0);
  }, 60_000);

  it("moves to another section when its link is clicked", async () => {
    // Routing inside the webview, which is the one thing a screenshot cannot tell you about.
    const before = await (await harness.session.findFilled(".content")).text();
    const link = await harness.session.find('a[href="#/servers"]');
    await link.click();
    // The same element, read again: the router replaces what is inside it.
    const after = await (await harness.session.findFilled(".content")).text();
    expect(after).not.toBe(before);
  }, 60_000);
});
