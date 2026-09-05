/**
 * A screen nothing mounts is indistinguishable from a screen nobody wrote.
 *
 * ⚠ **The fourth time this project has met that shape, and the second at this layer.** T366
 * found three commands registered and called from nowhere; T443 found a whole screen with no
 * way in; T479 found two core modules reachable only from their own tests, and built the
 * guard for that layer. On 2026-09-05 `CloseButton.tsx` turned out to be the same thing: it
 * existed, was worded in both languages, had three tests — and no screen rendered it. The
 * tests reached it by importing it themselves, so they passed while nobody could get to it.
 *
 * A test that mounts a component proves the component works. It proves nothing at all about
 * whether anybody can arrive at it, and it is the natural thing to write either way.
 *
 * **Two questions, because one of them was not enough.** The first is whether anything
 * imports the file. The second is whether anything actually puts it on the screen — added
 * after breaking the first on purpose: commenting out `<CloseButton />` while leaving its
 * import in place went straight past a guard that only followed imports.
 *
 * **The sources come from the bundler, not from the disk.** `import.meta.glob` is what Vite
 * already uses to resolve this project's modules, and reading through it keeps this file
 * inside the browser-side `tsconfig` — the alternative was `node:fs`, which would have meant
 * teaching the application's own type checking about Node.
 *
 * **What this does not check.** Usefulness: a component rendered behind a condition that is
 * never true passes here. That is a harder question; this one is cheap and has already found
 * two things.
 */

import { expect, it } from "vitest";

/** Where the outside world comes in. Everything worth reaching is reached from here. */
const DOORS = ["main.tsx"];

/**
 * Files nothing mounts yet, each with the reason and what closes it.
 *
 * **This list is the point of the check, and it must shrink.** An entry says "written, and
 * deliberately not connected yet, because —". An entry with no reason is a file somebody
 * forgot, which is exactly what this is here to find.
 */
const NOT_MOUNTED_YET: Array<[string, string]> = [
  [
    "features/convert/ValidationResult.tsx",
    "Found by this guard on its first run, 2026-09-05, together with the command it displays: " +
      "`convertValidate` is wrapped in ipc.ts and called by no screen either. FR-027 is met " +
      "without them — the preparation task validates on its own and fails the task on a bad " +
      "verdict, so a file that did not pass is never offered. What is missing is the ability " +
      "to check a file the application did not make: one copied from elsewhere, or produced " +
      "by the owner's own scripts, which this project deliberately keeps as a fallback path. " +
      "That is a screen somebody has to decide on, not a wire to reconnect. Closed by that " +
      "decision, or by deleting all three pieces.",
  ],
];

const RAW = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** Every source of the interface, keyed the way this file talks about paths: from `src`. */
const SOURCE = new Map<string, string>();
for (const [key, text] of Object.entries(RAW)) {
  const name = key.replace(/^\.\.\//, "");
  // Tests and their scaffolding are not screens: nothing in the application should reach
  // them, and a guard demanding it would ask for the opposite of what it wants.
  if (/(^|\/)__tests__\//.test(name)) continue;
  if (/(^|\/)test-[^/]+$/.test(name)) continue;
  SOURCE.set(name, text);
}

/**
 * The file with its block comments taken out.
 *
 * ⚠ **Without this the render check could not fail.** A JSX comment is written
 * `{` + `/* <Thing /> *` + `/}` — the element is still there, character for character, so
 * searching the raw text finds a component that was deliberately commented out and reports it
 * as rendered. Caught by breaking this very check on purpose, which is the only way it was
 * ever going to be caught.
 */
function withoutComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "");
}

/** Resolve a relative specifier against the file that wrote it. */
function resolveFrom(from: string, spec: string): string | null {
  const parts = from.split("/").slice(0, -1);
  for (const segment of spec.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") parts.pop();
    else parts.push(segment);
  }
  const base = parts.join("/");
  for (const candidate of [
    base,
    `${base}.tsx`,
    `${base}.ts`,
    `${base}/index.tsx`,
    `${base}/index.ts`,
  ]) {
    if (SOURCE.has(candidate)) return candidate;
  }
  return null;
}

/**
 * What a file imports, and which of those it loads lazily.
 *
 * Both shapes: a static `from "./x"` and a dynamic `import("./x")`. The second is how a
 * component loaded on demand is reached — `Mascot.tsx` does exactly that — and a walker that
 * knew only the first would report it stranded and teach people to write excuses.
 */
function importsOf(file: string): { all: string[]; lazy: string[] } {
  const all: string[] = [];
  const lazy: string[] = [];
  for (const m of (SOURCE.get(file) as string).matchAll(
    /(?:from\s+|\bimport\s*\(\s*)["'](\.[^"']+)["']/g,
  )) {
    const found = resolveFrom(file, m[1]);
    if (!found) continue;
    all.push(found);
    if (m[0].includes("import(")) lazy.push(found);
  }
  return { all, lazy };
}

/** Everything the application can arrive at, starting from the door. */
function reachable(): { seen: Set<string>; lazy: Set<string> } {
  const seen = new Set<string>();
  const lazy = new Set<string>();
  const queue = [...DOORS];
  while (queue.length > 0) {
    const file = queue.pop() as string;
    if (seen.has(file) || !SOURCE.has(file)) continue;
    seen.add(file);
    const found = importsOf(file);
    found.lazy.forEach((f) => lazy.add(f));
    queue.push(...found.all);
  }
  return { seen, lazy };
}

it("the walk starts from a door that exists", () => {
  // Everything below is a comparison against the set this walk produces. A door that resolved
  // to nothing would make that set empty, and an empty set agrees with every rule there is.
  for (const door of DOORS) expect(SOURCE.has(door), `${door} is not there`).toBe(true);
  expect(SOURCE.size).toBeGreaterThan(30);
  expect(reachable().seen.size).toBeGreaterThan(30);
});

it("every screen can be reached from the door", () => {
  const { seen } = reachable();
  const excused = new Set(NOT_MOUNTED_YET.map(([f]) => f));
  const stranded = [...SOURCE.keys()]
    .filter((f) => f.endsWith(".tsx"))
    .filter((f) => !seen.has(f))
    .filter((f) => !excused.has(f))
    .sort();

  expect(
    stranded,
    "these are written and nothing mounts them. A component with no way in is not a feature; " +
      "it cannot be found by using the application, because there is no path to it, and it " +
      "cannot be found by the screen tests, because they render it themselves.",
  ).toEqual([]);
});

it("every screen that is imported is also put on the screen", () => {
  // The half an import walk cannot see. Deleting `<CloseButton />` while leaving the import
  // line above it is an ordinary edit — a section commented out during a change and never put
  // back — and it leaves the file as reachable as ever.
  const { seen, lazy } = reachable();
  const code = new Map<string, string>();
  for (const f of seen) code.set(f, withoutComments(SOURCE.get(f) as string));

  const excused = new Set(NOT_MOUNTED_YET.map(([f]) => f));
  const never: string[] = [];
  for (const f of seen) {
    if (!f.endsWith(".tsx") || DOORS.includes(f) || excused.has(f)) continue;
    // A lazily loaded module is rendered under whatever local name the loader gave it, so its
    // own export name proves nothing. Reaching it dynamically is the evidence.
    if (lazy.has(f)) continue;

    const exported = [...(code.get(f) as string).matchAll(/export function ([A-Z][A-Za-z0-9]*)/g)];
    if (exported.length === 0) continue;

    const shown = exported.some(([, name]) =>
      [...seen].some(
        (other) => other !== f && new RegExp(`<${name}[\\s/>]`).test(code.get(other) as string),
      ),
    );
    if (!shown) never.push(f);
  }

  expect(
    never.sort(),
    "these are imported and never rendered. An import keeps a file alive in the graph and puts " +
      "nothing on anybody's screen.",
  ).toEqual([]);
});

it("the list of unmounted files does not rot", () => {
  // An excuse for a file that is now mounted — or now deleted — outlives the reason for it,
  // and the list stops being read.
  const { seen } = reachable();
  for (const [file, why] of NOT_MOUNTED_YET) {
    expect(why.length, `${file} is excused with no reason written down`).toBeGreaterThan(20);
    expect(SOURCE.has(file), `${file} is gone — take it off the list`).toBe(true);
    expect(seen.has(file), `${file} is mounted now — take it off the list`).toBe(false);
  }
});
