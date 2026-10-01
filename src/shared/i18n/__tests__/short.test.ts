/**
 * T674 — short interface. The owner, 2026-10-01: too much descriptive text that means nothing
 * to somebody who does not already understand it.
 *
 * The rule: screens carry short labels; explanations live only in errors, one line plus
 * «Подробнее». This holds it in numbers rather than in attention: every `ui.*` wording in both
 * languages is at most `UI_LIMIT` characters, and every error hint is one or two short phrases.
 *
 * **The exceptions are named one by one, each with its reason**, and the list shrinks on its
 * own: an exception that has become short enough, or whose key is gone, fails the last check.
 */

import { describe, expect, it } from "vitest";
import { ru } from "../ru";
import { en } from "../en";
import type { Catalogue, Lang } from "../catalogue";

const UI_LIMIT = 90;
const HINT_LIMIT = 110;

/** Legal texts are not ours to shorten (T674: «не трогать лицензию и „О программе“»). */
const LEGAL = "the licence and About are legal texts, left as they are (T674)";
/** Screens T673 takes out of the menu in favour of «Видео»: not cleaned, only kept green. */
const LEAVING = "a screen T673 takes out of the menu (Подготовка/Качества/Пакет/Заливка)";
/** Shown folded under «Подробнее», and what it says is a consequence a person is owed. */
const FOLDED = "folded under «Подробнее»: what a dangerous step will and will not do";

const EXCEPTIONS: Record<string, string> = {
  "ui.about.licenceBody1b": LEGAL,
  "ui.about.sourceMissing": LEGAL,
  "ui.about.thirdPartyBody": LEGAL,
  "ui.about.thirdPartyBodyTail": LEGAL,
  "ui.about.geoBody": LEGAL,
  "ui.about.geoBodyTail": LEGAL,

  "ui.deploy.replaceCaddyfileMeans": FOLDED,
  "ui.upgrade.rollBackKeeps": FOLDED,

  "ui.batch.explain": LEAVING,
  "ui.ladder.explain": LEAVING,
  "ui.ladder.formulaExplain": LEAVING,
  "ui.ladder.borrowExplain": LEAVING,
  "ui.ladder.measureExplain": LEAVING,
  "ui.ladder.measureChunks": LEAVING,
  "ui.ladder.measureAnchor": LEAVING,
  "ui.ladder.estimateFromThisMachine": LEAVING,
  "ui.ladder.estimateNotAsked": LEAVING,
  "ui.ladder.estimateFromModel": LEAVING,
  "ui.upload.notReady": LEAVING,
  "ui.upload.lead": LEAVING,
  "ui.upload.startedHint": LEAVING,
  "ui.validation.failed": LEAVING,
  "ui.convert.lossless": LEAVING,
  "ui.convert.startedHint": LEAVING,
  "ui.convert.nextHint": LEAVING,
  "ui.convert.nextCancelled": LEAVING,
};

const CATALOGUES: Array<[Lang, Catalogue]> = [
  ["ru", ru],
  ["en", en],
];

/** Every wording under `ui`, by its dotted name. A function is called with stand-in values. */
function wordings(catalogue: Catalogue): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  const walk = (node: unknown, path: string) => {
    if (typeof node === "string") out.push([path, node]);
    else if (typeof node === "function") {
      const fn = node as (...args: unknown[]) => unknown;
      out.push([path, String(fn(...Array.from({ length: fn.length }, () => 1)))]);
    } else if (node && typeof node === "object") {
      for (const [k, v] of Object.entries(node)) walk(v, `${path}.${k}`);
    }
  };
  walk(catalogue.ui, "ui");
  return out;
}

describe("short interface (T674)", () => {
  it.each(CATALOGUES)("no %s ui wording is longer than the limit", (_lang, catalogue) => {
    const all = wordings(catalogue);
    expect(all.length).toBeGreaterThan(400);
    const long = all
      .filter(([path, text]) => text.length > UI_LIMIT && !(path in EXCEPTIONS))
      .map(([path, text]) => `${path} (${text.length}): ${text}`);
    expect(
      long,
      `these ui wordings are longer than ${UI_LIMIT} characters. Cut them to a label, move the ` +
        "explanation into an error's «Подробнее», or — if it truly cannot be short — name it " +
        "in EXCEPTIONS with the reason",
    ).toEqual([]);
  });

  it.each(CATALOGUES)("every %s error hint is one or two short phrases", (_lang, catalogue) => {
    const long = Object.entries(catalogue.errors)
      .filter(([, w]) => w.hint.length > HINT_LIMIT || w.hint.split(/[.!?](\s|$)/).length > 5)
      .map(([code, w]) => `${code} (${w.hint.length}): ${w.hint}`);
    expect(long, "a hint says what to do, not how the application works").toEqual([]);
  });

  it("every exception still names a long wording that exists", () => {
    const stale: string[] = [];
    for (const path of Object.keys(EXCEPTIONS)) {
      const lengths = CATALOGUES.map(
        ([, c]) => wordings(c).find(([p]) => p === path)?.[1].length ?? -1,
      );
      if (lengths.every((n) => n <= UI_LIMIT)) stale.push(`${path} (${lengths.join("/")})`);
    }
    expect(stale, "these are gone or short now — take them out of EXCEPTIONS").toEqual([]);
  });
});
