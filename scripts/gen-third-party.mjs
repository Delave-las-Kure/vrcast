/**
 * T106 — сборка уведомлений о сторонних лицензиях в THIRD-PARTY.md.
 *
 * Почему сборкой, а не руками. Список зависимостей меняется при каждом обновлении,
 * и написанный однажды вручную он устареет в первый же месяц. Устаревший перечень
 * лицензий хуже отсутствующего: он выглядит выполненным обязательством, не будучи им.
 *
 * Почему на Node, а не на bash с jq или на Python. Node уже нужен для интерфейса
 * и стоит на всех машинах, где эта сборка идёт, включая непрерывную интеграцию.
 * Добавить ради одного сценария ещё и python или jq значило бы завести требование,
 * о котором узнают в тот день, когда его не окажется.
 *
 * Тексты лицензий берутся с диска — из исходников зависимостей, а не из своего
 * набора образцов: у многих пакетов в файле стоит имя правообладателя, и подменять
 * его типовым текстом нельзя, это и есть то самое уведомление, которое мы обязаны
 * сохранить.
 *
 * Запуск:
 *   node scripts/gen-third-party.mjs           — перезаписать THIRD-PARTY.md
 *   node scripts/gen-third-party.mjs --check    — упасть, если файл устарел
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, existsSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const APP = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(APP, "THIRD-PARTY.md");
const CHECK = process.argv.includes("--check");

/** Имена файлов, в которых пакеты держат текст лицензии. */
const LICENSE_FILE = /^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([-.].*)?$/i;

/** Прочитать тексты лицензий, лежащие в каталоге пакета. */
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
      // Заглушки вида «см. LICENSE-MIT» не несут уведомления и только раздувают файл.
      if (text.length > 200) out.push(text);
    } catch {
      /* нечитаемый файл — не повод рушить сборку перечня */
    }
  }
  return out;
}

/** Зависимости ядра из `cargo metadata`. */
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

/** Зависимости интерфейса, попадающие в поставку (без средств разработки). */
function nodePackages() {
  // Дерево берётся из package-lock.json, а не у `npm ls`. Причин две. Первая:
  // на Windows npm — это .cmd, и запускать его из Node уже нельзя, а через
  // оболочку аргументы не экранируются. Вторая важнее: замок и есть точное
  // описание того, что установится, — спрашивать о том же ещё и программу
  // значит завести второй источник ответа на один вопрос.
  const lockPath = join(APP, "package-lock.json");
  if (!existsSync(lockPath)) {
    throw new Error("нет package-lock.json — состав зависимостей интерфейса неизвестен");
  }
  const lock = JSON.parse(readFileSync(lockPath, "utf8"));
  if (!lock.packages) {
    throw new Error("package-lock.json старого образца: нет раздела packages");
  }

  const found = new Map();
  for (const [path, info] of Object.entries(lock.packages)) {
    // Корень — это мы сами, а средства разработки в поставку не попадают.
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
      /* пакет не установлен — покажем то, что знает замок */
    }
    found.set(`${name}@${version}`, { name, version, license, source, dir });
  }
  return [...found.values()];
}

/**
 * Раздел про вложенный FFmpeg.
 *
 * Его нет ни в дереве зависимостей ядра, ни интерфейса: он не библиотека, а готовая
 * программа, которую мы кладём в поставку целиком. Обязательство от этого не меньше,
 * а больше — мы раздаём чужой исполняемый файл под GPL, значит обязаны дать и его
 * исходный код. Сведения берутся из того же закреплённого описания, по которому
 * сборка скачивается, чтобы они не могли разойтись.
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

  // Нераспакованные исходники — не мелочь. Тексты лицензий берутся с диска,
  // и пакет без каталога молча выпал бы из перечня: файл собрался бы короче,
  // а сверка объявила бы его устаревшим, не сказав почему. Лучше отказаться
  // сразу и назвать причину.
  const пропали = [...rust, ...node].filter((p) => !p.dir || !existsSync(p.dir));
  if (пропали.length > 0) {
    const примеры = пропали.slice(0, 5).map((p) => `${p.name} ${p.version}`).join(", ");
    throw new Error(
      `исходников нет на диске у ${пропали.length} зависимостей (${примеры}). ` +
        "Перечень лицензий собирается из них, и без них он будет неполным. " +
        "Выполните `cargo fetch --manifest-path src-tauri/Cargo.toml` и `npm ci`.",
    );
  }

  // Тексты складываются по содержимому: у сотни пакетов лицензия дословно одна
  // и та же, и печатать её сто раз — значит сделать файл нечитаемым.
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
    const шапка =
      packages.length > 6
        ? `${packages.slice(0, 6).join(", ")} и ещё ${packages.length - 6}`
        : packages.join(", ");
    // Ограда длиннее самой длинной цепочки обратных кавычек внутри текста:
    // в лицензиях встречается разметка, и короткая ограда рассыпала бы вёрстку
    // всего файла начиная с этого места.
    const самая_длинная = Math.max(0, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
    const ограда = "`".repeat(Math.max(3, самая_длинная + 1));
    parts.push(`<details><summary>${шапка}</summary>`, "", ограда, text, ограда, "", "</details>", "");
  }

  return parts.join("\n");
}

const текст = build();

/**
 * Привести к одному виду концы строк.
 *
 * На Windows git выдаёт файл с иными концами строк, чем те, с которыми он лежит
 * в хранилище. Без этого сверка объявляла бы перечень устаревшим на каждой машине
 * с Windows — и была бы не проверкой, а помехой, которую первым делом отключат.
 */
const однообразно = (s) => s.replace(/\r\n/g, "\n").trim();

if (CHECK) {
  const было = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (однообразно(было) !== однообразно(текст)) {
    console.error(
      "THIRD-PARTY.md устарел: состав зависимостей изменился.\n" +
        "Выполните `npm run third-party` и включите обновлённый файл в коммит.",
    );
    process.exit(1);
  }
  console.log("THIRD-PARTY.md соответствует дереву зависимостей.");
} else {
  // Пишем с одинаковыми концами строк на любой системе: иначе файл, собранный
  // на Windows, отличался бы от собранного на Linux целиком, и разница в одну
  // зависимость терялась бы среди двадцати тысяч мнимых.
  writeFileSync(OUT, `${однообразно(текст)}\n`, "utf8");
  console.log(`THIRD-PARTY.md собран: ${OUT}`);
}
