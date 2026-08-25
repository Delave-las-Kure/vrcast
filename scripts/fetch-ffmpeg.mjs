/**
 * T115 — put FFmpeg beside the application AT BUILD TIME.
 *
 * Not on a person's machine (FR-112, R-01): a person installs the application and prepares a
 * video straight away. Downloading on the first run would mean a refusal wherever there is
 * no network, and different versions for different people — that is, a different result from
 * one and the same file.
 *
 * What is taken and why is in `ffmpeg.json` beside this file. Here there is only the
 * mechanism: download it, VERIFY THE SNAPSHOT and unpack the two programs we need.
 *
 * The snapshot check is required and cannot be switched off. We put somebody else's
 * executable into our package and sign our name under it; accepting "whatever came back from
 * the address" without checking would mean handing people whatever happens to be at the far
 * end on a bad day.
 *
 * Unpacking goes through `tar`: it is present in Windows 10+ and in every Linux, and it
 * reads both of our archive kinds. A library for unpacking would add a dependency for
 * something already installed on the system.
 *
 * To run it:
 *   node scripts/fetch-ffmpeg.mjs                    — download for this system
 *   node scripts/fetch-ffmpeg.mjs --for linux-x64    — download for another
 *   node scripts/fetch-ffmpeg.mjs --force            — download again
 *   node scripts/fetch-ffmpeg.mjs --check            — only check that it is in place
 *
 * `--for` is not there for convenience. Checking the Linux build from a developer's machine
 * runs in a container that has no Node, while the Tauri bundler refuses to build without the
 * bundled programs. Without a way to put somebody else's in place beforehand that check
 * would stop working at all — and would stop catching what it exists for.
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const APP = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = JSON.parse(readFileSync(join(APP, "scripts", "ffmpeg.json"), "utf8"));
const OUT_DIR = join(APP, "src-tauri", "binaries");

const FORCE = process.argv.includes("--force");
const CHECK = process.argv.includes("--check");

/** Which system to put it in place for. This one by default. */
const FOR = (() => {
  const i = process.argv.indexOf("--for");
  return i >= 0 ? process.argv[i + 1] : `${process.platform}-${process.arch}`;
})();

/** Platform triples by the manifest's keys: the bundler looks for the programs by these. */
const TRIPLES = {
  "win32-x64": "x86_64-pc-windows-msvc",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

/** The programs we need. The player from the archive is left out — a needless hundred megabytes. */
const WANTED = ["ffmpeg", "ffprobe"];

/**
 * The target platform's triple: Tauri looks for the bundled programs under exactly that name.
 *
 * For OUR OWN system it is asked of the compiler itself: the name has to match what the
 * bundler will use, or the program simply never reaches the installer — quietly, without a
 * single complaint. For another system it comes from the table: `rustc` cannot be asked about
 * one, and it still has to be put in place.
 */
function targetTriple(key) {
  const own = `${process.platform}-${process.arch}`;
  if (key !== own) return TRIPLES[key];

  try {
    const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
    const line = out.split("\n").find((l) => l.startsWith("host:"));
    if (line) return line.slice("host:".length).trim();
  } catch {
    /* no compiler at hand — the table will do */
  }
  return TRIPLES[key];
}

function platformKey() {
  if (!MANIFEST.platforms[FOR] || !TRIPLES[FOR]) {
    throw new Error(
      `no FFmpeg build is pinned for ${FOR}. Supported: ${Object.keys(MANIFEST.platforms).join(", ")}. ` +
        "The target systems are Windows and Linux (macOS was deferred by the owner's decision).",
    );
  }
  return FOR;
}

function outputPath(name, triple, suffix) {
  return join(OUT_DIR, `${name}-${triple}${suffix}`);
}

async function download(url, to) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`could not download ${url}: ${res.status} ${res.statusText}`);

  const total = Number(res.headers.get("content-length") ?? 0);
  const chunks = [];
  let got = 0;
  let shown = 0;
  for await (const chunk of res.body) {
    chunks.push(chunk);
    got += chunk.length;
    // A mark every ten per cent: a hundred and sixty megabytes do not arrive instantly, and
    // a silent build looks frozen.
    if (total && got - shown > total / 10) {
      shown = got;
      process.stdout.write(`  ${Math.round((got / total) * 100)} %\n`);
    }
  }
  const data = Buffer.concat(chunks);
  writeFileSync(to, data);
  return createHash("sha256").update(data).digest("hex");
}

/**
 * Unpack the archive.
 *
 * Plain `tar` will not do, and that is not pedantry. Different things may stand behind the
 * name `tar` on a system: in the Git shell it is GNU tar, which **does not read zip at all**
 * and needs a separate program for xz, while in Windows itself it is bsdtar, which reads
 * both of our kinds. Which one turns up depends on which shell it was started from — that
 * is, on chance.
 *
 * So they are tried in turn and the one that managed it is used. The Windows one comes first,
 * by its full path: looking for it by name is pointless, for exactly that confusion.
 */
function extract(archive, into) {
  const candidates =
    process.platform === "win32"
      ? [`${process.env.SystemRoot ?? "C:\\Windows"}\\System32\\tar.exe`, "tar"]
      : ["tar"];

  const problems = [];
  for (const tar of candidates) {
    try {
      execFileSync(tar, ["-xf", archive, "-C", into], { stdio: "pipe" });
      return;
    } catch (e) {
      problems.push(`${tar}: ${(e.stderr?.toString() || e.message).trim()}`);
    }
  }
  throw new Error(
    `not one of the available programs could unpack the archive.\n  ${problems.join("\n  ")}`,
  );
}

/** Find a file with the wanted name in the unpacked tree. */
function findFile(dir, name) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      const found = findFile(path, name);
      if (found) return found;
    } else if (entry.name === name) {
      return path;
    }
  }
  return null;
}

function report(triple, suffix) {
  let ok = true;
  for (const name of WANTED) {
    const path = outputPath(name, triple, suffix);
    if (existsSync(path)) {
      console.log(`  ${name}: ${(statSync(path).size / 1024 / 1024).toFixed(1)} MB`);
    } else {
      console.log(`  ${name}: MISSING`);
      ok = false;
    }
  }
  return ok;
}

async function main() {
  const key = platformKey();
  const entry = MANIFEST.platforms[key];
  const triple = targetTriple(key);

  if (CHECK) {
    console.log(`FFmpeg ${MANIFEST.version} for ${triple}:`);
    if (!report(triple, entry.exe_suffix)) {
      console.error("Run `npm run ffmpeg`, or the installer will be built without FFmpeg.");
      process.exit(1);
    }
    return;
  }

  const ready = WANTED.every((n) => existsSync(outputPath(n, triple, entry.exe_suffix)));
  if (ready && !FORCE) {
    console.log(`FFmpeg ${MANIFEST.version} is already in place:`);
    report(triple, entry.exe_suffix);
    return;
  }

  const url = `https://github.com/BtbN/FFmpeg-Builds/releases/download/${MANIFEST.release_tag}/${entry.asset}`;
  console.log(`Downloading ${entry.asset} (${MANIFEST.version})…`);

  const work = mkdtempSync(join(tmpdir(), "vrcast-ffmpeg-"));
  try {
    const archive = join(work, entry.asset);
    const got = await download(url, archive);

    if (got !== entry.sha256) {
      throw new Error(
        `the snapshot did not match.\n  expected: ${entry.sha256}\n  got:      ${got}\n` +
          "This is no trifle: somebody else's executable would go into our package under our " +
          "name. If the build was updated deliberately, write the new snapshot into " +
          "scripts/ffmpeg.json.",
      );
    }
    console.log("  the snapshot matched");

    extract(archive, work);

    mkdirSync(OUT_DIR, { recursive: true });
    for (const name of WANTED) {
      const file = `${name}${entry.exe_suffix}`;
      const found = findFile(work, file);
      if (!found) throw new Error(`the archive holds no ${file}`);
      copyFileSync(found, outputPath(name, triple, entry.exe_suffix));
    }

    console.log(`FFmpeg ${MANIFEST.version} is ready for ${triple}:`);
    report(triple, entry.exe_suffix);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

await main();
