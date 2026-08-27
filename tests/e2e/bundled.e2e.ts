/**
 * T347 — the FFmpeg that ships inside actually ships, and actually runs.
 *
 * **The recorded trap of `externalBin`, and it has two ends.** A program to be bundled has to
 * be named with the target triple where it is kept — `binaries/ffmpeg-x86_64-pc-windows-msvc.exe`
 * — and a file without that suffix is **silently left out**: nothing fails at build time. The
 * application installs, opens, and dies on the first file it is asked to prepare, in front of
 * somebody who has just installed it and has no idea what FFmpeg is.
 *
 * On the way out the bundler **strips the triple**, so beside the built application the names
 * are plain: `ffmpeg.exe`. Checking for the suffixed name there — which is what this file did
 * on its first draft — looks like a careful check and passes on nothing.
 *
 * So both ends are checked, because they are different conditions with different failures:
 * the suffix where the sources are kept, the plain name where the application will look. And
 * beyond being present, the programs are **run**: an executable bit lost in packaging looks
 * exactly like a missing file to everything except a directory listing.
 */

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { applicationPath } from "./session";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = join(HERE, "..", "..");

const WINDOWS = process.platform === "win32";
const TRIPLE = WINDOWS ? "x86_64-pc-windows-msvc" : "x86_64-unknown-linux-gnu";
const TAIL = WINDOWS ? ".exe" : "";

const PROGRAMS = ["ffmpeg", "ffprobe"];

describe("what ships beside the application", () => {
  it("keeps the sources under the name the bundler recognises", () => {
    // The suffix is not decoration: without it the file is not bundled, and it is not bundled
    // quietly. This is the end that fails invisibly.
    const kept = join(APP, "src-tauri", "binaries");
    const there = existsSync(kept) ? readdirSync(kept) : [];

    for (const name of PROGRAMS) {
      const wanted = `${name}-${TRIPLE}${TAIL}`;
      expect(
        there.includes(wanted),
        `${wanted} is not in src-tauri/binaries. What is: ${there.join(", ") || "nothing"}.\n` +
          "  Fetch them:  npm run ffmpeg",
      ).toBe(true);
    }
  });

  it("puts them beside the binary under the name it will look for", () => {
    // Beside it, not on the machine's PATH: an FFmpeg the developer happens to have installed
    // answers this question for them and for nobody else (FR-112). And under the plain name —
    // the triple is stripped on the way out.
    const beside = dirname(applicationPath());
    const there = readdirSync(beside).filter((f) => f.startsWith("ff"));

    for (const name of PROGRAMS) {
      const wanted = `${name}${TAIL}`;
      expect(
        there.includes(wanted),
        `${wanted} is not beside the application. What is: ${there.join(", ") || "nothing"}`,
      ).toBe(true);
    }
  });

  it("runs them from there", () => {
    // Present is not the same as runnable, and this is the half a directory listing cannot
    // answer.
    const beside = dirname(applicationPath());

    for (const name of PROGRAMS) {
      const said = execFileSync(join(beside, `${name}${TAIL}`), ["-version"], {
        encoding: "utf8",
        timeout: 30_000,
      });
      expect(said).toMatch(new RegExp(`^${name} version `));
    }
  }, 60_000);
});
