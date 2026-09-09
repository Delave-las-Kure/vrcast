/**
 * T566 — `.media__title` is protected against overflow the same way its siblings are.
 *
 * **Why this reads the file's text instead of rendering and checking a computed style.**
 * `vitest.config.ts` has no `css: true` and nothing else imports `styles.css` into jsdom for
 * a component test — `getComputedStyle` on a rendered `.media__title` would answer with the
 * browser's un-styled default, telling this test nothing real either way. Reading the actual
 * rule that ships is the direct question this task asks, in the same spirit as this
 * project's backend source-checks (`registry.rs`'s
 * `the_account_is_installed_beside_the_sweep_that_reads_it`, `task_purge.rs`'s
 * `purge_finished_before_is_called_at_startup` from the same audit round).
 *
 * **The `?raw` import, not `node:fs`** — the same reason `both-halves.test.ts` and
 * `reachable.test.ts` already give: reading through the bundler keeps this file inside the
 * browser-side `tsconfig` (`include: ["src"]`, no Node types), where `node:fs`/`node:path`
 * fail `tsc --noEmit` outright (confirmed: `Cannot find module 'node:fs'`, `Cannot find name
 * 'process'` — this project genuinely has no `@types/node` wired into `src/`'s type
 * checking, unlike `tests/e2e/`, which has its own separate `tsconfig.node.json`).
 *
 * **What was found before this was written.** `Media.title` has no length limit on the
 * backend (deliberately — see the backend test for T566, `tests/integration/library_ops.rs`:
 * a title never becomes a file name or a path, unlike `slug`, so there is no
 * filesystem-shaped ceiling to enforce), and `.media__title { font-weight: 600; }` carried
 * none of the overflow protection eight other places in this same file already have
 * (`.env-value`, `.step__detail`, `.server__facts dd`, `.file__name`, `.form__value`,
 * `.notice__details code`/`.notice__list code`, the diagnostics monospace block — all
 * `word-break: break-all` or `break-word`). A long, space-free title would sit inside
 * `.media__head`, a flex row with `justify-content: space-between`, unable to shrink or wrap
 * and pushing `.media__facts` aside or stretching the whole card sideways.
 */

import { describe, expect, it } from "vitest";
// A `?raw` import: the file's own text, resolved at build time — not a Node `fs` read,
// which `src/`'s browser-side `tsconfig` has no types for (confirmed: `tsc --noEmit` fails
// on `node:fs`/`process` here, unlike `tests/e2e/`, which has its own `tsconfig.node.json`).
// Needs one line of `vitest.config.ts` alongside it: Vitest stubs every `.css` import as
// empty by default regardless of a `?raw` suffix, so `css.include` there narrowly turns
// real CSS text back on for exactly this query pattern.
import styles from "../styles.css?raw";

/** The text of one CSS rule block, `.selector { ... }`, isolated by brace matching so a
 *  later block sharing a substring of the name cannot be mistaken for it (`.media__title`
 *  vs. a hypothetical `.media__title--something` is exactly the kind of near-miss brace
 *  matching avoids that a naive `indexOf` + fixed-length slice would not). */
function ruleBody(css: string, selector: string): string {
  const start = css.indexOf(`${selector} {`);
  if (start === -1) {
    throw new Error(`no rule for ${selector} was found in styles.css`);
  }
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  if (close === -1) {
    throw new Error(`the rule for ${selector} has no closing brace`);
  }
  return css.slice(open + 1, close);
}

describe(".media__title overflow protection (T566)", () => {
  it("breaks a long, space-free title instead of letting it overflow the card", () => {
    const rule = ruleBody(styles, ".media__title");
    expect(rule).toMatch(/overflow-wrap\s*:\s*break-word/);
  });

  it("can actually shrink inside its flex row — the wrap rule alone does nothing without it", () => {
    // `.media__head` is `display: flex`, and a flex item's default `min-width` is `auto`,
    // which refuses to shrink narrower than the item's own content. Without `min-width: 0`
    // on `.media__title` itself, `overflow-wrap: break-word` has no room to ever act in —
    // the two together are what actually protects the layout, and a rule with only the
    // first would pass a naive check while still overflowing in a real browser.
    const rule = ruleBody(styles, ".media__title");
    expect(rule).toMatch(/min-width\s*:\s*0/);
  });

  it("still carries its own styling — this is a check of what was added, not a rewrite", () => {
    const rule = ruleBody(styles, ".media__title");
    expect(rule).toMatch(/font-weight\s*:\s*600/);
  });
});
