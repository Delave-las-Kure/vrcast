/**
 * T115 — положить FFmpeg рядом с приложением ПРИ СБОРКЕ.
 *
 * Не у пользователя (FR-112, R-01): человек ставит приложение и сразу готовит видео.
 * Скачивание при первом запуске означало бы отказ там, где сети нет, и разные версии
 * у разных людей — то есть разный результат на одном и том же файле.
 *
 * Что берётся и почему — в `ffmpeg.json` рядом. Здесь только механика: скачать,
 * СВЕРИТЬ СЛЕПОК и распаковать две нужные программы.
 *
 * Слепок обязателен и не подлежит отключению. Мы кладём чужой исполняемый файл
 * в свою поставку и подписываемся под ним своим именем; принять «то, что пришло
 * по адресу» без сверки значило бы раздать людям что угодно, что окажется на том
 * конце в неудачный день.
 *
 * Распаковка через `tar`: он есть и в Windows 10+, и в любом Linux, и читает
 * оба наших вида архива. Библиотека для распаковки добавила бы зависимость ради
 * того, что уже стоит в системе.
 *
 * Запуск:
 *   node scripts/fetch-ffmpeg.mjs                    — скачать под эту систему
 *   node scripts/fetch-ffmpeg.mjs --for linux-x64    — скачать под другую
 *   node scripts/fetch-ffmpeg.mjs --force            — скачать заново
 *   node scripts/fetch-ffmpeg.mjs --check            — только проверить, что на месте
 *
 * `--for` нужен не для удобства. Проверка сборки под Linux с машины разработчика
 * идёт в контейнере, где Node нет, а сборщик Tauri отказывается собирать без
 * вложенных программ. Без возможности положить чужие заранее эта проверка
 * перестала бы работать вовсе — и перестала бы ловить то, ради чего заведена.
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

/** Под какую систему класть. По умолчанию — под эту. */
const FOR = (() => {
  const i = process.argv.indexOf("--for");
  return i >= 0 ? process.argv[i + 1] : `${process.platform}-${process.arch}`;
})();

/** Тройки платформ по ключам описания: сборщик ищет программы именно по ним. */
const TRIPLES = {
  "win32-x64": "x86_64-pc-windows-msvc",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

/** Программы, которые нам нужны. Проигрыватель из архива не берём — он лишние сто мегабайт. */
const WANTED = ["ffmpeg", "ffprobe"];

/**
 * Тройка целевой платформы: Tauri ищет вложенные программы именно по такому имени.
 *
 * Для СВОЕЙ системы спрашивается у самого компилятора: имя обязано совпасть с тем,
 * что подставит сборщик, иначе программа просто не попадёт в установщик — молча,
 * без единой жалобы. Для чужой берётся из таблицы: `rustc` о ней не спросишь,
 * а класть под неё всё равно надо.
 */
function targetTriple(key) {
  const own = `${process.platform}-${process.arch}`;
  if (key !== own) return TRIPLES[key];

  try {
    const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
    const line = out.split("\n").find((l) => l.startsWith("host:"));
    if (line) return line.slice("host:".length).trim();
  } catch {
    /* компилятора рядом нет — обойдёмся таблицей */
  }
  return TRIPLES[key];
}

function platformKey() {
  if (!MANIFEST.platforms[FOR] || !TRIPLES[FOR]) {
    throw new Error(
      `для ${FOR} сборка FFmpeg не закреплена. Поддерживаются: ${Object.keys(MANIFEST.platforms).join(", ")}. ` +
        "Целевые системы — Windows и Linux (macOS отложена решением владельца).",
    );
  }
  return FOR;
}

function outputPath(name, triple, suffix) {
  return join(OUT_DIR, `${name}-${triple}${suffix}`);
}

async function download(url, to) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`не скачать ${url}: ${res.status} ${res.statusText}`);

  const total = Number(res.headers.get("content-length") ?? 0);
  const chunks = [];
  let got = 0;
  let shown = 0;
  for await (const chunk of res.body) {
    chunks.push(chunk);
    got += chunk.length;
    // Отметки раз в десять процентов: сто шестьдесят мегабайт идут не мгновенно,
    // и молчащая сборка выглядит зависшей.
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
 * Распаковать архив.
 *
 * Просто `tar` не годится, и это не придирка. Под именем `tar` в системе может
 * оказаться разное: в оболочке Git это GNU tar, который **не читает zip вовсе**
 * и требует отдельной программы для сжатия xz, а в самой Windows — bsdtar,
 * который читает оба наших вида. Какая из них попадётся, зависит от того, из какой
 * оболочки запущено, — то есть от случая.
 *
 * Поэтому перебираем по очереди и берём ту, что справилась. Windows-овская идёт
 * первой по полному пути: искать её по имени бессмысленно, ровно из-за этой путаницы.
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
    `архив не распаковать ни одной из доступных программ.\n  ${problems.join("\n  ")}`,
  );
}

/** Найти в распакованном дереве файл с нужным именем. */
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
      console.log(`  ${name}: ${(statSync(path).size / 1024 / 1024).toFixed(1)} МБ`);
    } else {
      console.log(`  ${name}: НЕТ`);
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
    console.log(`FFmpeg ${MANIFEST.version} для ${triple}:`);
    if (!report(triple, entry.exe_suffix)) {
      console.error("Выполните `npm run ffmpeg`, иначе установщик соберётся без FFmpeg.");
      process.exit(1);
    }
    return;
  }

  const готово = WANTED.every((n) => existsSync(outputPath(n, triple, entry.exe_suffix)));
  if (готово && !FORCE) {
    console.log(`FFmpeg ${MANIFEST.version} уже на месте:`);
    report(triple, entry.exe_suffix);
    return;
  }

  const url = `https://github.com/BtbN/FFmpeg-Builds/releases/download/${MANIFEST.release_tag}/${entry.asset}`;
  console.log(`Скачиваем ${entry.asset} (${MANIFEST.version})…`);

  const work = mkdtempSync(join(tmpdir(), "vrcast-ffmpeg-"));
  try {
    const archive = join(work, entry.asset);
    const got = await download(url, archive);

    if (got !== entry.sha256) {
      throw new Error(
        `слепок не сошёлся.\n  ожидался: ${entry.sha256}\n  получен:  ${got}\n` +
          "Это не мелочь: в поставку попал бы чужой исполняемый файл под нашим именем. " +
          "Если сборка обновилась намеренно — впишите новый слепок в scripts/ffmpeg.json.",
      );
    }
    console.log("  слепок сошёлся");

    extract(archive, work);

    mkdirSync(OUT_DIR, { recursive: true });
    for (const name of WANTED) {
      const файл = `${name}${entry.exe_suffix}`;
      const found = findFile(work, файл);
      if (!found) throw new Error(`в архиве нет ${файл}`);
      copyFileSync(found, outputPath(name, triple, entry.exe_suffix));
    }

    console.log(`FFmpeg ${MANIFEST.version} готов для ${triple}:`);
    report(triple, entry.exe_suffix);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

await main();
