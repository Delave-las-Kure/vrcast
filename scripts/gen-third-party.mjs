/**
 * T106 — assembling the third-party licence notices into THIRD-PARTY.md.
 *
 * Why assembled rather than written by hand. The list of dependencies changes with every
 * upgrade, and written once by hand it goes stale within the first month. A stale list of
 * licences is worse than a missing one: it looks like an obligation met while not being one.
 *
 * Why Node rather than bash with jq, or Python. Node is already needed for the interface and
 * is installed on every machine this build runs on, continuous integration included. Adding
 * python or jq for one script would create a requirement people learn about on the day it is
 * not there.
 *
 * The licence texts are taken from disk — from the dependencies' sources rather than from a
 * set of templates of our own: many packages hold a copyright holder's name in that file,
 * and replacing it with a boilerplate text is not allowed; it is the very notice we are
 * obliged to keep.
 *
 * A NOTE ON THE LANGUAGE. Everything this script emits INTO THIRD-PARTY.md stays in Russian:
 * that document is a notice for the people the application is handed to, not code, and its
 * language is a decision of its own. Everything else — the comments, the names, what goes to
 * the console — is English, like the rest of the code.
 *
 * To run it:
 *   node scripts/gen-third-party.mjs           — rewrite THIRD-PARTY.md
 *   node scripts/gen-third-party.mjs --check    — fail if the file is stale
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, existsSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const APP = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(APP, "THIRD-PARTY.md");
const CHECK = process.argv.includes("--check");

/** The names packages keep their licence text under. */
const LICENSE_FILE = /^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([-.].*)?$/i;

/** Read the licence texts lying in a package's directory. */
function licenseTexts(dir) {
  if (!dir || !existsSync(dir)) return [];
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return [];
  }
  const out = [];
  for (const name of entries) {
    if (!LICENSE_FILE.test(name)) continue;
    const path = join(dir, name);
    try {
      if (!statSync(path).isFile()) continue;
      const text = readFileSync(path, "utf8").trim();
      // Stubs like "see LICENSE-MIT" carry no notice and only bloat the file.
      if (text.length > 200) out.push(text);
    } catch {
      /* an unreadable file is no reason to break assembling the list */
    }
  }
  return out;
}

/** The core's dependencies, from `cargo metadata`. */
function rustPackages() {
  const raw = execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--all-features", "--manifest-path", join(APP, "src-tauri", "Cargo.toml")],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  const meta = JSON.parse(raw);
  const own = new Set(meta.workspace_members);
  return meta.packages
    .filter((p) => !own.has(p.id))
    .map((p) => ({
      name: p.name,
      version: p.version,
      license: p.license ?? (p.license_file ? "см. файл лицензии" : "не указана"),
      source: p.repository ?? null,
      dir: p.manifest_path ? dirname(p.manifest_path) : null,
    }));
}

/** The interface's dependencies that reach the package (development tools aside). */
function nodePackages() {
  // The tree comes from package-lock.json rather than from `npm ls`. For two reasons. The
  // first: on Windows npm is a .cmd, which can no longer be started from Node, and going
  // through a shell leaves the arguments unescaped. The second matters more: the lock file
  // is the exact description of what will be installed — asking a program the same question
  // would create a second source of one answer.
  const lockPath = join(APP, "package-lock.json");
  if (!existsSync(lockPath)) {
    throw new Error("there is no package-lock.json — the interface's dependencies are unknown");
  }
  const lock = JSON.parse(readFileSync(lockPath, "utf8"));
  if (!lock.packages) {
    throw new Error("package-lock.json is of the old shape: it has no packages section");
  }

  const found = new Map();
  for (const [path, info] of Object.entries(lock.packages)) {
    // The root is ourselves, and development tools do not reach the package.
    if (path === "" || info.dev || info.devOptional) continue;
    const cut = path.lastIndexOf("node_modules/");
    if (cut < 0) continue;
    const name = path.slice(cut + "node_modules/".length);
    const version = info.version ?? "?";

    const dir = join(APP, ...path.split("/"));
    let license = typeof info.license === "string" ? info.license : "не указана";
    let source = null;
    try {
      const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
      if (pkg.license) {
        license = typeof pkg.license === "string" ? pkg.license : (pkg.license.type ?? license);
      }
      const url = typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
      source = url ? url.replace(/^git\+/, "").replace(/\.git$/, "") : null;
    } catch {
      /* the package is not installed — we show what the lock file knows */
    }
    found.set(`${name}@${version}`, { name, version, license, source, dir });
  }
  return [...found.values()];
}

/**
 * The section about the bundled FFmpeg.
 *
 * It is in neither the core's dependency tree nor the interface's: it is not a library but a
 * finished program we put into the package whole. That makes the obligation greater rather
 * than smaller — we hand out somebody else's executable under the GPL, so we owe its source
 * code too. The details come from the same pinned manifest the build is downloaded by, so
 * that the two cannot diverge.
 */
function ffmpegSection() {
  const m = JSON.parse(readFileSync(join(APP, "scripts", "ffmpeg.json"), "utf8"));
  return [
    "## FFmpeg — в поставке целиком",
    "",
    `Версия: **${m.version}** (${m.source.license}).`,
    "",
    "Приложение не скачивает FFmpeg при первом запуске: он кладётся в установщик",
    "при сборке. Исходный код именно этой сборки:",
    "",
    `- исходники FFmpeg: [${m.source.ffmpeg_commit}](${m.source.ffmpeg_source_url})`,
    `- сценарии сборки: [${m.release_tag}](${m.source.build_scripts_url})`,
    "",
    "Оба адреса ведут на неизменяемые точки, а не на главную ветку, которая уедет.",
    "",
  ].join("\n");
}

function table(rows) {
  const lines = ["| Пакет | Версия | Лицензия | Источник |", "|---|---|---|---|"];
  for (const r of rows.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))) {
    const src = r.source ? `[источник](${r.source})` : "—";
    lines.push(`| \`${r.name}\` | ${r.version} | ${r.license} | ${src} |`);
  }
  return lines.join("\n");
}

function build() {
  const rust = rustPackages();
  const node = nodePackages();

  // Unpacked sources are no trifle. The licence texts are taken from disk, and a package with
  // no directory would quietly drop out of the list: the file would come out shorter, and the
  // comparison would declare it stale without saying why. Better to refuse at once and name
  // the reason.
  const missing = [...rust, ...node].filter((p) => !p.dir || !existsSync(p.dir));
  if (missing.length > 0) {
    const examples = missing.slice(0, 5).map((p) => `${p.name} ${p.version}`).join(", ");
    throw new Error(
      `the sources of ${missing.length} dependencies are not on disk (${examples}). ` +
        "The list of licences is assembled from them, and without them it would be incomplete. " +
        "Run `cargo fetch --manifest-path src-tauri/Cargo.toml` and `npm ci`.",
    );
  }

  // The texts are gathered by their contents: a hundred packages carry word-for-word the same
  // licence, and printing it a hundred times would make the file unreadable.
  const texts = new Map();
  for (const p of [...rust, ...node]) {
    for (const text of licenseTexts(p.dir)) {
      const key = createHash("sha256").update(text).digest("hex");
      if (!texts.has(key)) texts.set(key, { text, packages: [] });
      texts.get(key).packages.push(`${p.name} ${p.version}`);
    }
  }

  const parts = [
    "# Сторонние компоненты",
    "",
    "Этот файл собирается командой `npm run third-party` из дерева зависимостей,",
    "а не пишется руками: список меняется при каждом обновлении, и написанный",
    "однажды вручную он устареет в первый же месяц. Устаревший перечень лицензий",
    "хуже отсутствующего — он выглядит выполненным обязательством, не будучи им.",
    "",
    "Само приложение распространяется под GPL-3.0-or-later (см. [LICENSE](LICENSE)).",
    "Перечисленное ниже — чужие работы, входящие в поставку, с их собственными",
    "условиями. В поставку входит также сборка FFmpeg; её условия и способ получения",
    "исходного кода описаны в разделе «О программе» самого приложения.",
    "",
    ffmpegSection(),
    `## Ядро (Rust) — ${rust.length}`,
    "",
    table(rust),
    "",
    `## Интерфейс (npm) — ${node.length}`,
    "",
    table(node),
    "",
    `## Тексты лицензий — ${texts.size}`,
    "",
    "Одинаковые тексты сведены вместе: у многих пакетов лицензия дословно совпадает.",
    "",
  ];

  for (const { text, packages } of [...texts.values()].sort((a, b) =>
    a.packages[0].localeCompare(b.packages[0]),
  )) {
    const heading =
      packages.length > 6
        ? `${packages.slice(0, 6).join(", ")} и ещё ${packages.length - 6}`
        : packages.join(", ");
    // The fence is longer than the longest run of backticks inside the text: licences do hold
    // markup, and a short fence would scatter the layout of the whole file from that point on.
    const longestRun = Math.max(0, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
    const fence = "`".repeat(Math.max(3, longestRun + 1));
    parts.push(`<details><summary>${heading}</summary>`, "", fence, text, fence, "", "</details>", "");
  }

  return parts.join("\n");
}

const document = build();

/**
 * Bring the line endings to one form.
 *
 * On Windows git hands back a file with line endings other than the ones it is stored with.
 * Without this the comparison would declare the list stale on every Windows machine — and
 * would be not a check but a nuisance, the first thing anyone switches off.
 */
const uniform = (s) => s.replace(/\r\n/g, "\n").trim();

if (CHECK) {
  const before = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (uniform(before) !== uniform(document)) {
    console.error(
      "THIRD-PARTY.md is stale: the set of dependencies changed.\n" +
        "Run `npm run third-party` and include the updated file in the commit.",
    );
    process.exit(1);
  }
  console.log("THIRD-PARTY.md matches the dependency tree.");
} else {
  // Written with the same line endings on any system: otherwise a file assembled on Windows
  // would differ from one assembled on Linux in its entirety, and a difference of one
  // dependency would be lost among twenty thousand imaginary ones.
  writeFileSync(OUT, `${uniform(document)}\n`, "utf8");
  console.log(`THIRD-PARTY.md assembled: ${OUT}`);
}
