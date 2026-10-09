/**
 * A tour of the whole interface, screen by screen, for a person to look at — not a check.
 *
 * The owner asked to go through every step of working with a video and a server and see
 * what is awkward or wrong. This walks the real window (the e2e build, a throwaway server in
 * Docker) through the sections in the order work goes through them and, at every step,
 * leaves the window's picture and the screen's text side by side:
 *
 *   <shots>/<S><NN>-<step>.png   the window (and .p2.png, .p3.png … when the page scrolls)
 *   <shots>/<S><NN>-<step>.txt   document.body.innerText at that moment
 *   <shots>/index.md             one line per picture: number, step, what was done before it
 *
 * `S` is the section: A first start and every section empty, B the server wizard, C the
 * server's card (upgrade, deploy screen), D «Video» from a file to a link, E the library,
 * F viewers and limits, G diagnostics, H tasks / appearance / about, I the server gone,
 * J the key screens in English, Z cleaning up (no pictures).
 *
 * **Run in parts.** Each section starts the application on the same data directory and the
 * same container, so `VRCAST_TOUR=A,B` now and `VRCAST_TOUR=C` later carry on from each
 * other (A starts from an empty directory and removes the containers of a previous tour).
 * Without `VRCAST_TOUR` every section runs, in order.
 *
 *   VRCAST_E2E_TARGET_DIR  where the e2e build is (see video.e2e.ts for how to build it)
 *   VRCAST_TOUR_SHOTS      the pictures (default: a folder in the system's temporary one)
 *   VRCAST_TOUR_STATE      the data directory, films and scratch (default: likewise)
 *
 * **A step that does not work is written down, not fatal.** It is pictured as it is, its
 * line in index.md says what went wrong, and the tour goes on — what is broken is part of
 * what the tour is for. Only the harness itself failing stops a section.
 *
 * Skipped — said out loud — without the e2e binary, Docker or the test image, or off Windows.
 */

import { spawnSync } from "node:child_process";
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, it } from "vitest";

import { Harness } from "./session";
import type { Session } from "./webdriver";
import { ensureDriver } from "../../scripts/fetch-webdriver.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = join(HERE, "..", "..");

const IMAGE = "vrcast-test-sshd:5";
const CLEAN_IMAGE = "vrcast-test-clean:1";
const MARKER = "VRCAST-E2E-BUILD-a7c3f19e";
const KEY = join(APP, "src-tauri", "tests", "fixtures", "encrypted_ed25519.key");
const PASSPHRASE = "test-passphrase-1234";
/** The clean image lets root in by password (fixtures/docker-clean/Dockerfile). */
const CLEAN_PASSWORD = "test-container-password";

/** Fixed ports, so a stopped and started container answers where the profile says. */
const SSH_PORT = 47022;
const HTTP_PORT = 47080;
const CLEAN_SSH_PORT = 47122;
const NAME = "vrcast-e2e-tour";
const CLEAN_NAME = "vrcast-e2e-tour-clean";
const VIEWER_FAST = "vrcast-e2e-tour-viewer-fast";
const VIEWER_SLOW = "vrcast-e2e-tour-viewer-slow";

const SHOTS = process.env.VRCAST_TOUR_SHOTS ?? join(tmpdir(), "vrcast-tour-shots");
const STATE = process.env.VRCAST_TOUR_STATE ?? join(tmpdir(), "vrcast-tour-state");
const DATA = join(STATE, "data");
const FILMS = join(STATE, "films");

const FILM1 = join(FILMS, "Фильм с двумя дорожками.mkv");
const FILM2 = join(FILMS, "Короткий 720p.mp4");
const BROKEN = join(FILMS, "Битый файл.mp4");
const SINGLE = join(FILMS, "single-clip.mp4");
const STRAY = join(FILMS, "stray-file.mp4");

const SECTIONS: Record<string, string> = {
  A: "Первый запуск, каждый раздел пустым",
  B: "Мастер добавления сервера",
  C: "Карточка сервера: состояние, обновление, развёртывание",
  D: "«Видео»: от файла до ссылки — начало",
  D2: "«Видео»: после перезапуска приложения — до ссылки; занятое имя",
  E: "Библиотека",
  F: "Зрители и ограничения качества",
  G: "Диагностика",
  H: "Задачи, оформление, «О программе»",
  I: "Сервер недоступен",
  J: "Ключевые экраны на английском",
  Z: "Уборка (без снимков)",
};

const wanted = new Set(
  (process.env.VRCAST_TOUR ?? Object.keys(SECTIONS).join(","))
    .toUpperCase()
    .split(/[^A-Z0-9]+/)
    .filter(Boolean),
);

// ---------- what is needed, and whether it is here ----------

function e2eBinary(): string | null {
  if (process.env.VRCAST_E2E_BINARY) return process.env.VRCAST_E2E_BINARY;
  const dir = process.env.VRCAST_E2E_TARGET_DIR;
  return dir ? join(dir, "release", "vrcast-studio.exe") : null;
}

function carriesMarker(path: string): boolean {
  const needle = Buffer.from(MARKER);
  const fd = openSync(path, "r");
  try {
    const block = Buffer.alloc(8 << 20);
    let carry = Buffer.alloc(0);
    let at = 0;
    const size = statSync(path).size;
    while (at < size) {
      const n = readSync(fd, block, 0, block.length, at);
      if (n <= 0) break;
      const hay = Buffer.concat([carry, block.subarray(0, n)]);
      if (hay.includes(needle)) return true;
      carry = hay.subarray(Math.max(0, hay.length - needle.length));
      at += n;
    }
    return false;
  } finally {
    closeSync(fd);
  }
}

function docker(args: string[]) {
  return spawnSync("docker", args, { encoding: "utf8" });
}

const why: string[] = [];
const binary = e2eBinary();
if (process.platform !== "win32") why.push(`not Windows (${process.platform})`);
if (!binary || !existsSync(binary))
  why.push(`no e2e binary (${binary ?? "VRCAST_E2E_TARGET_DIR unset"})`);
else if (!carriesMarker(binary)) why.push(`${binary} is NOT the e2e build — refusing to drive it`);
if (docker(["info", "--format", "{{.ServerVersion}}"]).status !== 0)
  why.push("Docker is not running");
else if (docker(["image", "inspect", IMAGE]).status !== 0) why.push(`no image ${IMAGE}`);
if (!existsSync(KEY)) why.push(`no test key ${KEY} (run the Rust integration tests once)`);
if (why.length > 0) console.warn(`tour.e2e skipped: ${why.join("; ")}`);

// ---------- small things ----------

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function message(e: unknown): string {
  const text = (e instanceof Error ? e.message : String(e)).split("\n")[0];
  return text.split(". Last:")[0].split(" (Session info")[0].slice(0, 240);
}

/** A string as an XPath literal, apostrophes and all. */
function xq(text: string): string {
  if (!text.includes("'")) return `'${text}'`;
  return `concat('${text.split("'").join(`', "'", '`)}')`;
}

// ---------- the record: pictures, texts, index.md ----------

interface Shot {
  file: string;
  step: string;
  did: string;
  pages: number;
}
interface SectionLog {
  shots: Shot[];
  notes: string[];
}

let sec = "A";
const logs: Record<string, SectionLog> = {};

function logPath(letter: string) {
  return join(SHOTS, "log", `${letter}.json`);
}

/** «A» pictures are A01-…, «D2» ones D2-01-… — so no section's names run into another's. */
function prefix(letter: string): string {
  return letter.length > 1 ? `${letter}-` : letter;
}

function readLog(letter: string): SectionLog | null {
  if (logs[letter]) return logs[letter];
  try {
    return JSON.parse(readFileSync(logPath(letter), "utf8")) as SectionLog;
  } catch {
    return null;
  }
}

/** A section begins afresh: its earlier pictures and lines go. */
function beginSection(letter: string) {
  sec = letter;
  mkdirSync(join(SHOTS, "log"), { recursive: true });
  for (const f of readdirSync(SHOTS)) {
    if (new RegExp(`^${prefix(letter)}\\d\\d-`).test(f)) rmSync(join(SHOTS, f), { force: true });
  }
  logs[letter] = { shots: [], notes: [] };
  shotNo[letter] = 0;
  saveLog();
}

function saveLog() {
  writeFileSync(logPath(sec), JSON.stringify(logs[sec], null, 2));
  writeIndex();
}

function note(text: string) {
  logs[sec].notes.push(text);
  saveLog();
  console.log(`tour ${sec}: ${text}`);
}

function writeIndex() {
  const lines: string[] = [
    "# Обход интерфейса VRCast Studio",
    "",
    `Снимки окна (1280×800) и текст экрана рядом (.txt — document.body.innerText). Если экран ` +
      `прокручивается, к снимку добавлены продолжения .p2.png, .p3.png … Обновлено: ` +
      `${new Date().toISOString().replace("T", " ").slice(0, 19)} UTC.`,
    "",
  ];
  let n = 0;
  for (const letter of Object.keys(SECTIONS)) {
    const log = readLog(letter);
    if (!log) continue;
    if (log.shots.length === 0 && log.notes.length === 0) continue;
    lines.push(`## ${letter}. ${SECTIONS[letter]}`, "");
    if (log.shots.length > 0) {
      lines.push("| № | Снимок | Шаг | Что сделано перед снимком |", "|---|---|---|---|");
      for (const shot of log.shots) {
        n += 1;
        const more = shot.pages > 1 ? ` (+${shot.pages - 1} прокр.)` : "";
        lines.push(
          `| ${n} | [${shot.file}](${shot.file}.png)${more} | ${shot.step} | ${shot.did.replace(/\|/g, "/").replace(/\n/g, " ")} |`,
        );
      }
      lines.push("");
    }
    if (log.notes.length > 0) {
      lines.push("Заметки по ходу:", "");
      for (const x of log.notes) lines.push(`- ${x}`);
      lines.push("");
    }
  }
  writeFileSync(join(SHOTS, "index.md"), lines.join("\n"));
}

// ---------- the application ----------

let nativeDriver = "";
let harness: Harness | undefined;

function s(): Session {
  if (!harness) throw new Error("the application is not running");
  return harness.session;
}

function ours(): { pid: number; path: string }[] {
  const dir = dirname(resolve(binary as string));
  const ps = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-Command",
      `Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith('${dir.replace(/'/g, "''")}', [System.StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { "$($_.ProcessId)|$($_.ExecutablePath)" }`,
    ],
    { encoding: "utf8" },
  );
  return ps.stdout
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => {
      const [pid, path] = l.split("|");
      return { pid: Number(pid), path };
    });
}

function killOurs() {
  for (const p of ours()) {
    try {
      process.kill(p.pid, "SIGKILL");
    } catch {
      // gone already
    }
  }
}

async function driverDown() {
  const until = Date.now() + 15_000;
  while (Date.now() < until) {
    try {
      await fetch("http://127.0.0.1:4444/status");
    } catch {
      return;
    }
    await sleep(200);
  }
}

async function launch() {
  mkdirSync(DATA, { recursive: true });
  mkdirSync(join(STATE, "tmp"), { recursive: true });
  const env = {
    VRCAST_DATA_DIR: DATA,
    VRCAST_E2E_VERIFY_ORIGIN: `http://127.0.0.1:${HTTP_PORT}`,
    TMP: join(STATE, "tmp"),
    TEMP: join(STATE, "tmp"),
  };
  harness = await Harness.start(nativeDriver, env, binary as string);
  await s().findFilled(".content");
  try {
    await s().setWindowRect(1280, 800);
  } catch (e) {
    note(`окно не удалось сделать 1280×800 — снято как есть (${message(e)})`);
  }
  if (!existsSync(join(DATA, "vrcast-studio.sqlite"))) {
    throw new Error(`the application did not write into ${DATA} — stopping before anything else`);
  }
}

async function shutdown() {
  try {
    await harness?.stop();
  } catch {
    // the window may be gone already
  }
  harness = undefined;
  killOurs();
  await driverDown();
}

async function ensureApp() {
  if (harness) return;
  await launch();
}

async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const r = await s().executeAsync<{ ok?: T; err?: unknown }>(
    `const done = arguments[arguments.length - 1];
     window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1])
       .then((ok) => done({ ok }), (err) => done({ err }));`,
    [cmd, args],
  );
  if (r.err !== undefined) throw new Error(`${cmd}: ${JSON.stringify(r.err)}`);
  return r.ok as T;
}

async function setLanguage(lang: "ru" | "en") {
  const settings = await invoke<Record<string, unknown>>("settings_get");
  if (settings.language === lang) return;
  await invoke("settings_set", { settings: { ...settings, language: lang } });
  await s().execute("location.reload()");
  await s().findFilled(".content");
}

async function pageText(): Promise<string> {
  return await s().execute<string>("return document.body.innerText");
}

async function until<T>(
  what: string,
  probe: () => Promise<T | null | false | undefined>,
  ms: number,
  every = 250,
): Promise<T> {
  const stop = Date.now() + ms;
  let last: unknown = null;
  for (;;) {
    try {
      const v = await probe();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    if (Date.now() > stop) {
      throw new Error(
        `${what}: not within ${Math.round(ms / 1000)} s${last ? ` (${message(last)})` : ""}`,
      );
    }
    await sleep(every);
  }
}

async function waitText(re: RegExp, ms = 30_000) {
  await until(`the screen to say ${re}`, async () => re.test(await pageText()), ms);
}

const LOADING =
  /Загружаем…|Читаем [^\n]*…|Смотрю, что за сервер…|Начинаю смотреть…|Спрашиваю сервер…|Считаю, что останется у зрителя…|Loading…/;

/** Let a screen arrive: a moment for the fade, then until nothing says it is still loading. */
async function settle(ms = 20_000) {
  await sleep(700);
  try {
    await until("the screen to load", async () => !LOADING.test(await pageText()), ms);
  } catch {
    // pictured as it is — the line says so if it matters
  }
  await sleep(300);
}

async function go(route: string) {
  await (await s().find(`a[href="#/${route}"]`)).click();
  await settle();
}

/** A button or a link by its words, waiting for it to be there and enabled. */
async function press(label: string, scope = "", ms = 15_000) {
  const el = await s().findX(
    `${scope}//*[self::button or self::a or self::summary][normalize-space()=${xq(label)} and not(@disabled)]`,
    ms,
  );
  await el.click();
}

async function has(xpath: string): Promise<boolean> {
  return await s().execute<boolean>(
    `return !!document.evaluate(arguments[0], document, null, 9, null).singleNodeValue;`,
    [xpath],
  );
}

async function field(label: string, value: string) {
  const input = await s().findX(`//label[span[normalize-space()=${xq(label)}]]//input`);
  await input.clear();
  await input.type(value);
}

async function chooseX(selectXpath: string, optionXpath: string) {
  const sel = await s().findX(selectXpath);
  await sel.click();
  await (await s().findX(`${selectXpath}/${optionXpath}`)).click();
}

// ---------- the pictures ----------

const shotNo: Record<string, number> = {};

async function snap(step: string, did: string) {
  shotNo[sec] = (shotNo[sec] ?? 0) + 1;
  const file = `${prefix(sec)}${String(shotNo[sec]).padStart(2, "0")}-${step}`;
  let pages = 0;
  let text = "";
  try {
    writeFileSync(join(SHOTS, `${file}.png`), Buffer.from(await s().screenshot(), "base64"));
    pages = 1;
    text = await pageText();
    // A screen longer than the window: the rest of it, a window at a time.
    const tall = await s().execute<{ h: number; c: number; top: number } | null>(
      `const m = document.querySelector('.content');
       return m ? { h: m.scrollHeight, c: m.clientHeight, top: m.scrollTop } : null;`,
    );
    if (tall && tall.h > tall.c + 8) {
      for (let y = tall.top + tall.c - 40; y < tall.h && pages < 5; y += tall.c - 40) {
        await s().execute(`document.querySelector('.content').scrollTop = arguments[0];`, [y]);
        await sleep(200);
        pages += 1;
        writeFileSync(
          join(SHOTS, `${file}.p${pages}.png`),
          Buffer.from(await s().screenshot(), "base64"),
        );
      }
      await s().execute(`document.querySelector('.content').scrollTop = arguments[0];`, [tall.top]);
    }
  } catch (e) {
    did = `${did} — СНИМОК НЕ ПОЛУЧИЛСЯ: ${message(e)}`;
  }
  writeFileSync(join(SHOTS, `${file}.txt`), `${did}\n\n---\n\n${text}`);
  logs[sec].shots.push({ file, step, did, pages });
  saveLog();
}

/** Do something, then picture the result — and if it did not work, say so on the line. */
async function step(name: string, did: string, act: () => Promise<void>): Promise<boolean> {
  let ok = true;
  try {
    await act();
  } catch (e) {
    ok = false;
    did = `${did} — НЕ ПОЛУЧИЛОСЬ: ${message(e)}`;
    note(`${name}: ${message(e)}`);
  }
  await snap(name, did);
  return ok;
}

// ---------- the servers ----------

async function answers(
  p: number,
  first: (c: ReturnType<typeof createConnection>) => void,
  ok: (t: string) => boolean,
) {
  return await new Promise<boolean>((done) => {
    const c = createConnection({ host: "127.0.0.1", port: p });
    let said = "";
    const end = (v: boolean) => {
      c.destroy();
      done(v);
    };
    c.setTimeout(2000, () => end(false));
    c.on("connect", () => first(c));
    c.on("data", (b) => {
      said += String(b);
      if (ok(said)) end(true);
    });
    c.on("error", () => end(false));
    c.on("close", () => done(ok(said)));
  });
}

async function sshAnswers(port: number) {
  return await answers(
    port,
    () => {},
    (t) => t.startsWith("SSH-"),
  );
}

async function httpAnswers(port: number) {
  return await answers(
    port,
    (c) => c.write("GET / HTTP/1.0\r\nHost: x\r\n\r\n"),
    (t) => t.startsWith("HTTP/"),
  );
}

function containerState(name: string): "running" | "stopped" | "absent" {
  const r = docker(["inspect", "-f", "{{.State.Running}}", name]);
  if (r.status !== 0) return "absent";
  return r.stdout.trim() === "true" ? "running" : "stopped";
}

async function waitServer() {
  await until(
    "the server container to answer",
    async () => (await sshAnswers(SSH_PORT)) && (await httpAnswers(HTTP_PORT)),
    60_000,
    300,
  );
}

async function ensureServer() {
  const state = containerState(NAME);
  if (state === "running") return;
  if (state === "stopped") {
    docker(["start", NAME]);
  } else {
    const dir = join(APP, "src-tauri", "tests", "fixtures", "docker");
    const built = docker(["build", "-t", IMAGE, "-f", join(dir, "Dockerfile"), dir]);
    if (built.status !== 0) throw new Error(`the image would not build: ${built.stderr}`);
    const run = docker([
      ...["run", "-d", "--name", NAME],
      ...["-p", `127.0.0.1:${SSH_PORT}:22`, "-p", `127.0.0.1:${HTTP_PORT}:80`],
      IMAGE,
    ]);
    if (run.status !== 0) throw new Error(`the container would not start: ${run.stderr}`);
  }
  await waitServer();
}

/** A container's address on Docker's network (the old `.NetworkSettings.IPAddress` is empty
 *  on current Docker). */
function ipOf(name: string): string {
  return docker([
    ...["inspect", "-f", "{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}", name],
  ])
    .stdout.trim()
    .split(/\s+/)[0];
}

function serverIp(): string {
  return ipOf(NAME);
}

function removeContainers() {
  for (const n of [VIEWER_FAST, VIEWER_SLOW, CLEAN_NAME, NAME]) docker(["rm", "-f", n]);
}

// ---------- the films ----------

function ffmpeg(): string {
  return join(APP, "src-tauri", "binaries", "ffmpeg-x86_64-pc-windows-msvc.exe");
}

function run(args: string[]) {
  const r = spawnSync(ffmpeg(), ["-nostdin", "-y", "-v", "error", ...args], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`ffmpeg: ${r.stderr}`);
}

function ensureFilms() {
  mkdirSync(FILMS, { recursive: true });
  const x264 = [
    "-c:v",
    "libx264",
    "-preset",
    "veryfast",
    "-crf",
    "21",
    "-g",
    "60",
    "-pix_fmt",
    "yuv420p",
  ];
  if (!existsSync(FILM1)) {
    // 1080p, 60 s, two audio tracks: English first, Russian SECOND — the one to choose.
    run([
      ...["-f", "lavfi", "-i"],
      "testsrc2=s=1920x1080:r=30:d=60,noise=alls=6:allf=t+u:enable='between(t,20,40)',noise=alls=18:allf=t+u:enable='gte(t,40)'",
      ...["-f", "lavfi", "-i", "sine=f=440:r=48000:d=60"],
      ...["-f", "lavfi", "-i", "sine=f=660:r=48000:d=60"],
      ...["-map", "0:v", "-map", "1:a", "-map", "2:a"],
      ...x264,
      ...["-c:a", "aac", "-b:a", "160k"],
      ...["-metadata:s:a:0", "language=eng", "-metadata:s:a:0", "title=English"],
      ...["-metadata:s:a:1", "language=rus", "-metadata:s:a:1", "title=Русский"],
      // Different every time it is made: a film measured once is not measured again, and
      // the tour wants to show «Measuring».
      ...["-metadata", `comment=tour-${Date.now()}`],
      FILM1,
    ]);
  }
  if (!existsSync(FILM2)) {
    run([
      ...["-f", "lavfi", "-i", "testsrc2=s=1280x720:r=30:d=20"],
      ...["-f", "lavfi", "-i", "sine=f=500:r=48000:d=20"],
      ...x264,
      ...["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"],
      FILM2,
    ]);
  }
  if (!existsSync(BROKEN)) {
    // An «mp4» that is not one: a header-shaped start and noise after it.
    const junk = Buffer.alloc(2 << 20);
    for (let i = 0; i < junk.length; i++) junk[i] = (i * 2654435761) >>> 24;
    Buffer.from("\0\0\0\x18ftypmp42").copy(junk, 0);
    writeFileSync(BROKEN, junk);
  }
  if (!existsSync(SINGLE)) {
    run([
      ...["-f", "lavfi", "-i", "testsrc2=s=1280x720:r=30:d=12"],
      ...["-f", "lavfi", "-i", "sine=f=300:r=48000:d=12"],
      ...x264,
      ...["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"],
      SINGLE,
    ]);
  }
  if (!existsSync(STRAY)) {
    run([
      ...["-f", "lavfi", "-i", "testsrc2=s=640x360:r=25:d=6"],
      ...x264,
      ...["-movflags", "+faststart"],
      STRAY,
    ]);
  }
}

// ---------- «Video» cards ----------

interface Card {
  state: string;
  stage: string | null;
  text: string;
}

async function card(id: string): Promise<Card | null> {
  return await s().execute<Card | null>(
    `const el = document.querySelector('[data-testid="video-' + arguments[0] + '"]');
     if (!el) return null;
     const cur = el.querySelector('.video__stage--current');
     return { state: (el.className.match(/video--(\\w+)/) || [])[1], stage: cur ? cur.dataset.stage : null, text: el.innerText };`,
    [id],
  );
}

async function cardUntil(id: string, what: string, ok: (c: Card) => boolean, ms: number) {
  return await until(
    what,
    async () => {
      const c = await card(id);
      return c && ok(c) ? c : null;
    },
    ms,
    150,
  );
}

const inCard = (id: string) => `//li[@data-testid='video-${id}']`;

interface Added {
  added: { id: string; source_path: string }[];
  refused: { path: string }[];
}

async function activeServerId(): Promise<string> {
  const servers = await invoke<{ id: string; is_active: boolean; name: string }[]>("servers_list");
  const active = servers.find((x) => x.is_active) ?? servers[0];
  if (!active) throw new Error("no server profile");
  return active.id;
}

// ---------- what one part of the tour leaves for the next ----------

interface TourState {
  id1: string;
  id2: string;
  caught: string[];
  paused: boolean;
  tasksShot: boolean;
  secondDone: boolean;
}

function saveTour(t: TourState) {
  writeFileSync(join(STATE, "tour.json"), JSON.stringify(t, null, 2));
}

function readTour(): TourState | null {
  try {
    return JSON.parse(readFileSync(join(STATE, "tour.json"), "utf8")) as TourState;
  } catch {
    return null;
  }
}

/**
 * The system's file dialog answered by the test: its next `open` gives back `paths`.
 *
 * The dialog is a window of the system, not of the page, and WebDriver cannot press it. The
 * page asks for it through `__TAURI_INTERNALS__.invoke('plugin:dialog|open')`, so that one
 * call is answered here and every other goes through untouched — the button, the screen and
 * what it does with the answer are the application's own. Returns false where the page
 * would not take the substitute (then the caller says so and falls back to the command).
 */
async function answerDialog(paths: string[] | string): Promise<string> {
  const how = await s().execute<string>(
    `const I = window.__TAURI_INTERNALS__;
     window.__tourPick = arguments[0];
     if (window.__tourHow) return window.__tourHow;
     const take = () => { const p = window.__tourPick; window.__tourPick = undefined; return p; };
     const orig = I.invoke;
     const wrapped = function (cmd, args, opts) {
       if (cmd === 'tour|probe') return Promise.resolve('ok');
       if (cmd === 'plugin:dialog|open' && window.__tourPick !== undefined) return Promise.resolve(take());
       return orig.apply(this, arguments);
     };
     try { I.invoke = wrapped; } catch (e) {}
     if (I.invoke === wrapped) return (window.__tourHow = 'assign');
     try { Object.defineProperty(I, 'invoke', { value: wrapped, configurable: true, writable: true }); } catch (e) {}
     if (I.invoke === wrapped) return (window.__tourHow = 'define');
     // The IPC itself: on Windows every call is a POST to http://ipc.localhost/<command>.
     const f = window.fetch;
     const answer = (v) => Promise.resolve(new Response(JSON.stringify(v), {
       status: 200,
       headers: { 'Content-Type': 'application/json', 'Tauri-Response': 'ok' },
     }));
     window.fetch = function (input, init) {
       const url = String(input && input.url ? input.url : input);
       if (/tour(%7C|\\|)probe/i.test(url)) return answer('ok');
       if (window.__tourPick !== undefined && /plugin(%3A|:)dialog(%7C|\\|)open/i.test(url)) {
         return answer(take());
       }
       return f.apply(this, arguments);
     };
     return (window.__tourHow = 'fetch');`,
    [paths],
  );
  // Proved before anything is pressed: a substitute that does not work would let the real
  // dialog open, and a system window nobody can press holds the tour up for good.
  try {
    const said = await s().executeAsync<string>(
      `const done = arguments[arguments.length - 1];
       window.__TAURI_INTERNALS__.invoke('tour|probe', {}).then(done, () => done('no'));
       setTimeout(() => done('timeout'), 3000);`,
    );
    return said === "ok" ? how : "";
  } catch {
    return "";
  }
}

/** Errors the page throws or logs from now on — for a screen that goes blank. */
async function catchErrors() {
  await s().execute(
    `window.__tourErrors = [];
     if (!window.__tourCatching) {
       window.__tourCatching = true;
       window.addEventListener('error', (e) => window.__tourErrors.push('error: ' + e.message));
       window.addEventListener('unhandledrejection', (e) => window.__tourErrors.push('rejection: ' + String(e.reason)));
       const was = console.error;
       console.error = function (...a) { window.__tourErrors.push('console: ' + a.map(String).join(' ')); return was.apply(this, a); };
     }`,
  );
}

async function caughtErrors(): Promise<string[]> {
  try {
    return await s().execute<string[]>("return window.__tourErrors || []");
  } catch {
    return [];
  }
}

/**
 * Watch the first video's card until it is done (or `deadline`): a picture of every stage as
 * it begins, «Pause»/«Resume» once during encoding, «Tasks» and the leave question once while
 * work is going on, and the second video cancelled in the middle of its work and removed.
 */
async function watchVideos(tour: TourState, deadline: number) {
  const { id1, id2 } = tour;
  const caught = new Set(tour.caught);
  const keep = () => {
    tour.caught = [...caught];
    saveTour(tour);
  };
  for (;;) {
    const c1 = await card(id1);
    const c2 = await card(id2);
    if (c1?.state === "done") break;
    if (c1?.state === "problem") {
      await snap("first-problem", "Первая карточка встала на проблеме");
      note(`первая карточка — проблема: ${c1.text.replace(/\n/g, " / ").slice(0, 300)}`);
      break;
    }
    if (Date.now() > deadline) {
      note(
        `к концу этого прогона первое видео ещё в работе: ${c1?.state}/${c1?.stage} — продолжение в следующей части`,
      );
      break;
    }
    if (c1?.state === "working" && c1.stage && !caught.has(c1.stage)) {
      caught.add(c1.stage);
      keep();
      if (c1.stage === "encoding") {
        // «rung k of n» comes with the first progress of the encode.
        try {
          await cardUntil(
            id1,
            "rung k of n",
            (c) => /ступень \d+ из \d+/.test(c.text) || c.stage !== "encoding",
            20_000,
          );
        } catch {
          // pictured without it
        }
      }
      await snap(`stage-${c1.stage}`, `Первая карточка: начался этап «${c1.stage}»`);
      continue;
    }
    if (c1?.state === "working" && c1.stage === "encoding" && !tour.paused) {
      tour.paused = true;
      keep();
      await step("paused", "«Пауза» во время кодирования", async () => {
        await press("Пауза", inCard(id1));
        await cardUntil(id1, "paused", (c) => c.state === "paused", 20_000);
      });
      await step("resumed", "«Продолжить»", async () => {
        await press("Продолжить", inCard(id1));
        await cardUntil(id1, "going again", (c) => c.state === "working", 20_000);
      });
      continue;
    }
    if (c1?.state === "working" && caught.size > 0 && !tour.tasksShot) {
      tour.tasksShot = true;
      keep();
      await step("tasks-during-work", "(раздел H) «Задачи», пока видео в работе", async () => {
        await go("tasks");
        await sleep(800);
      });
      await step(
        "leave-confirm",
        "(раздел H) Вопрос при выходе с идущей задачей — вызван событием app:quit-requested из теста (то же, что «Выйти» в меню значка в трее)",
        async () => {
          await invoke("plugin:event|emit", { event: "app:quit-requested", payload: null });
          await s().find('[data-testid="leave-confirm"]');
          await sleep(1000);
        },
      );
      try {
        await (await s().find('[data-testid="leave-no"]')).click();
      } catch (e) {
        note(`«Остаться» не нажалось: ${message(e)}`);
      }
      await go("video");
      continue;
    }
    if (!tour.secondDone && c2 && c2.state === "working" && c2.stage) {
      tour.secondDone = true;
      keep();
      await sleep(2500);
      const at = (await card(id2))?.stage ?? c2.stage;
      await step("second-working", `Второе видео в работе (этап «${at}»)`, async () => {});
      await step("second-cancel", "«Отмена» у второго видео", async () => {
        await press("Отмена", inCard(id2));
        await cardUntil(id2, "cancelled", (c) => c.state === "cancelled", 60_000);
      });
      await step("second-removed", "«Убрать» у отменённого второго видео", async () => {
        await press("Убрать", inCard(id2));
        await until("the card to go", async () => !(await card(id2)), 30_000);
      });
      continue;
    }
    await sleep(150);
  }
  keep();
  note(`этапы первой карточки, снятые к этому моменту: ${[...caught].join(", ") || "ни одного"}`);
}

// ---------- the tour ----------

describe.skipIf(why.length > 0)("a tour of the interface, with pictures", () => {
  beforeAll(async () => {
    mkdirSync(SHOTS, { recursive: true });
    mkdirSync(STATE, { recursive: true });
    nativeDriver = await ensureDriver();
    // Made once and kept in the state folder: the first run spends a minute on them.
    ensureFilms();
    // Nothing of an earlier run of the e2e build may hold the driver's port or the data.
    killOurs();
    await driverDown();
  }, 120_000);

  afterAll(async () => {
    await shutdown();
    writeIndex();
    console.log(`tour.e2e: pictures and index.md in ${SHOTS}`);
  }, 60_000);

  it.skipIf(!wanted.has("A"))(
    "A — the first start, and every section while there is nothing",
    async () => {
      beginSection("A");
      await shutdown();
      removeContainers();
      rmSync(DATA, { recursive: true, force: true });
      await launch();
      await settle();
      await snap("first-start", "Первый запуск: папка данных пустая, ничего не нажато");
      const lang = await s().execute<string>("return document.documentElement.lang || ''");
      const russian = /Серверы/.test(await pageText());
      if (!russian) {
        await step(
          "language-ru",
          `Интерфейс открылся не по-русски (lang=${lang}); в боковой панели выбран «Русский»`,
          async () => {
            await chooseX("(//nav[contains(@class,'sidebar')]//select)[1]", "option[@value='ru']");
            await waitText(/Серверы/);
          },
        );
      } else {
        note(`интерфейс сам открылся по-русски (язык системы; lang="${lang}")`);
      }
      const sections: [string, string][] = [
        ["servers", "Серверы"],
        ["library", "Библиотека"],
        ["video", "Видео"],
        ["viewers", "Зрители"],
        ["limits", "Ограничения"],
        ["diagnostics", "Диагностика"],
        ["tasks", "Задачи"],
        ["appearance", "Оформление"],
        ["about", "О программе"],
      ];
      for (const [route, title] of sections) {
        await step(`empty-${route}`, `Пустое состояние: пункт меню «${title}»`, async () => {
          await go(route);
        });
      }
    },
    240_000,
  );

  it.skipIf(!wanted.has("B"))(
    "B — the server wizard, step by step, with a typical mistake",
    async () => {
      beginSection("B");
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      // From no servers at all, whatever an earlier run left.
      for (const p of await invoke<{ id: string }[]>("servers_list")) {
        await invoke("server_remove", { id: p.id, confirmed: true });
      }
      await go("servers");
      const card =
        "//li[contains(@class,'server')][.//span[@class='server__name' and normalize-space()='Контейнер']]";
      const wizardIdle = async () =>
        !/Проверяем…/.test(await pageText()) &&
        ((await has("//button[normalize-space()='Дальше' and not(@disabled)]")) ||
          (await has("//code[contains(@class,'fingerprint')]")));
      await step(
        "wizard-open",
        "«Серверы» → «Добавить сервер»: шаг «Данные», пустая форма (вверху — найденный server.env владельца; «Подставить» НЕ нажимали)",
        async () => {
          await press("Добавить сервер");
          await s().find(".wizard");
          await sleep(800);
        },
      );
      await step(
        "wizard-filled-wrong-port",
        `Заполнено: Название «Контейнер», Адрес 127.0.0.1, Порт 47099 (ОШИБКА: там никто не слушает; верный ${SSH_PORT}), Домен stream.example.com, ключ, парольная фраза тоже НЕВЕРНАЯ`,
        async () => {
          await field("Название", "Контейнер");
          await field("Адрес", "127.0.0.1");
          await field("Порт", "47099");
          await field("Домен раздачи", "stream.example.com");
          await field("Путь к приватному ключу", KEY);
          await field("Парольная фраза ключа", "wrong-passphrase");
        },
      );
      await step("wizard-error-port", "«Дальше» с неверным портом — ответ мастера", async () => {
        await press("Дальше");
        await sleep(500);
        await until("the wizard to answer", wizardIdle, 90_000, 300);
      });
      const afterError = await invoke<{ name: string }[]>("servers_list");
      note(
        `после ошибки порта (мастер ещё открыт) профилей в базе уже ${afterError.length}: ${afterError.map((p) => `«${p.name}»`).join(", ")}`,
      );
      await step(
        "wizard-port-fixed",
        `Порт исправлен на ${SSH_PORT} прямо в мастере, снова «Дальше»`,
        async () => {
          await field("Порт", String(SSH_PORT));
          await press("Дальше");
          await sleep(500);
          await until("the wizard to answer", wizardIdle, 90_000, 300);
        },
      );
      if (await has("//code[contains(@class,'fingerprint')]")) {
        note("после исправления порта мастер пошёл дальше, к отпечатку");
      } else {
        await step(
          "wizard-cancelled",
          "Мастер дальше не пускает — «Отмена»: список серверов",
          async () => {
            await press("Отмена", "//div[contains(@class,'wizard')]");
            await until("the wizard to close", async () => !(await s().has(".wizard")), 15_000);
            await settle();
            await sleep(1500);
          },
        );
        await step(
          "list-after-revisit",
          "Ушли в «Библиотеку» и вернулись в «Серверы» — список перечитан",
          async () => {
            await go("library");
            await go("servers");
            await sleep(2000);
          },
        );
        await step("edit-open", "У «Контейнер» нажато «Изменить» — форма правки", async () => {
          await press("Изменить", card);
          await s().find(".wizard");
          await sleep(500);
        });
        await step(
          "edit-port-fingerprint",
          `В правке порт → ${SSH_PORT}, «Сохранить» → отпечаток (адрес изменился)`,
          async () => {
            await field("Порт", String(SSH_PORT));
            await press("Сохранить");
            await s().findX("//code[contains(@class,'fingerprint')]", 60_000);
          },
        );
      }
      await step("checking", "«Отпечаток верный» — сразу после нажатия", async () => {
        await press("Отпечаток верный");
        await sleep(300);
      });
      await step(
        "check-wrong-passphrase",
        "Проверка закончилась (парольная фраза ключа неверная)",
        async () => {
          await s().findX(
            "//section[contains(@class,'wizard__stage')]//button[normalize-space()='Готово' and not(@disabled)]",
            120_000,
          );
        },
      );
      const fixInWizard =
        "//section[contains(@class,'wizard__stage')]//button[normalize-space()='Исправить данные']";
      if (await has(fixInWizard)) {
        // T708: the passphrase is fixed where the mistake showed, in the wizard itself.
        await step(
          "wizard-fix-passphrase",
          "«Исправить данные» → введена верная парольная фраза → «Дальше» (тот же профиль, отпечаток второй раз не спрашивают)",
          async () => {
            await press("Исправить данные", "//section[contains(@class,'wizard__stage')]");
            const input = await s().findX(
              "//label[span[normalize-space()='Парольная фраза ключа']]//input",
              15_000,
            );
            await input.clear();
            await input.type(PASSPHRASE);
            await press("Дальше");
            await s().findX(
              "//section[contains(@class,'wizard__stage')]//button[normalize-space()='Готово' and not(@disabled)]",
              120_000,
            );
          },
        );
        await step("check-done-list", "«Готово» — список серверов", async () => {
          await press("Готово", "//section[contains(@class,'wizard__stage')]");
          await until("the wizard to close", async () => !(await s().has(".wizard")), 15_000);
          await settle();
          await sleep(2000);
        });
      } else {
        await step("check-done-list", "«Готово» — список серверов", async () => {
          await press("Готово", "//section[contains(@class,'wizard__stage')]");
          await until("the wizard to close", async () => !(await s().has(".wizard")), 15_000);
          await settle();
          await sleep(2000);
        });
        await step(
          "edit-passphrase",
          "«Изменить» → в форме правки введена верная парольная фраза",
          async () => {
            await press("Изменить", card);
            const input = await s().findX(
              "//label[span[normalize-space()='Парольная фраза ключа']]//input",
            );
            await input.clear();
            await input.type(PASSPHRASE);
          },
        );
        await step("edit-saved", "«Сохранить» — правка закрылась", async () => {
          await press("Сохранить");
          await until("the edit to close", async () => !(await s().has(".wizard")), 30_000);
          await settle();
          await sleep(2000);
        });
      }
      await step("test-ok", "«Проверить подключение» на карточке — результат", async () => {
        await press("Проверить подключение", card);
        await s().findX(`${card}//ol[@class='steps']/li`, 90_000);
        await s().findX(
          `${card}//button[normalize-space()='Проверить подключение' and not(@disabled)]`,
          90_000,
        );
      });
      const profiles =
        await invoke<{ name: string; is_active: boolean; host_fingerprint: string | null }[]>(
          "servers_list",
        );
      note(
        `профилей в конце: ${profiles.length} — ${profiles.map((p) => `«${p.name}»${p.is_active ? " (активный)" : ""}${p.host_fingerprint ? "" : " (без отпечатка)"}`).join(", ")}`,
      );
      if (!profiles.some((p) => p.is_active) && profiles[0]) {
        await step("made-active", "Активного сервера нет — нажато «Сделать активным»", async () => {
          await press("Сделать активным", card);
          await sleep(1500);
        });
      }
    },
    480_000,
  );

  it.skipIf(!wanted.has("C"))(
    "C — the server's card: state, versions, upgrade, deploy screen",
    async () => {
      beginSection("C");
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      await go("servers");
      await sleep(2000);
      await snap("card", "«Серверы»: карточка сервера (состояние, версии, кнопки)");

      // A second profile left by the wizard's mistake, if there is one: removed by hand.
      const strays =
        await invoke<{ id: string; name: string; host_fingerprint: string | null }[]>(
          "servers_list",
        );
      const unconfirmed = strays.filter((p) => !p.host_fingerprint);
      if (unconfirmed.length > 0) {
        const xp = "//li[contains(@class,'server')][.//p[contains(@class,'server__warning')]]";
        await step(
          "stray-remove-ask",
          `Лишний профиль без отпечатка (остался от ошибки в мастере) → «Удалить» — вопрос`,
          async () => {
            await press("Удалить", `(${xp})[1]`);
          },
        );
        await step("stray-removed", "«Да, удалить» — список", async () => {
          await press("Да, удалить", `(${xp})[1]`);
          await settle();
          await sleep(1000);
        });
      }

      let upgradeOffered = await has("//button[normalize-space()='Обновить серверную часть']");
      const stateFile = "/etc/vrcast/state.json";
      let downgraded = false;
      if (!upgradeOffered) {
        // The versions match, so the card offers nothing. To see the upgrade screen at all the
        // server is made to say it is one version behind (only its state file), and put back
        // after.
        note(
          `версии совпали (1 и 1) — чтобы показать экран обновления, в ${stateFile} контейнера версия временно понижена до 0`,
        );
        docker([
          ...["exec", NAME, "sed", "-i"],
          's/"vrcast_server_version": 1/"vrcast_server_version": 0/',
          stateFile,
        ]);
        downgraded = true;
        await go("library");
        await go("servers");
        await sleep(2500);
        await snap(
          "card-behind",
          "Сервер «отстаёт» на версию (state.json понижен тестом) — карточка",
        );
        upgradeOffered = await has("//button[normalize-space()='Обновить серверную часть']");
      }
      if (upgradeOffered) {
        await step(
          "upgrade-dialog",
          "«Обновить серверную часть» — что изменится, что сохранится (до согласия)",
          async () => {
            await press("Обновить серверную часть");
            await until(
              "the upgrade plan",
              async () =>
                (await has("//button[normalize-space()='Согласен, обновлять']")) ||
                /Версия \d+ → \d+|менять нечего/.test(await pageText()),
              90_000,
            );
          },
        );
        await step("upgrade-cancel", "«Отмена» в обновлении — карточка", async () => {
          await press("Отмена");
          await settle();
        });
      } else {
        note(
          "кнопки «Обновить серверную часть» нет и при «отставшем» сервере: приложение ждёт версию 1, а 0 читается как нечитаемый файл состояния → «Чужая раздача» (снимок card-behind); экран обновления в этой версии не достать",
        );
      }
      if (downgraded) {
        docker([
          ...["exec", NAME, "sed", "-i"],
          's/"vrcast_server_version": 0/"vrcast_server_version": 1/',
          stateFile,
        ]);
      }

      // A clean server, the way one comes from a hosting provider: the deploy screen.
      if (docker(["image", "inspect", CLEAN_IMAGE]).status !== 0) {
        note(`образа ${CLEAN_IMAGE} нет — экран развёртывания не снят`);
        return;
      }
      docker(["rm", "-f", CLEAN_NAME]);
      const started = docker([
        ...["run", "-d", "--name", CLEAN_NAME, "--privileged", "--cgroupns=private"],
        ...["--tmpfs", "/run", "--tmpfs", "/run/lock"],
        ...["-p", `127.0.0.1:${CLEAN_SSH_PORT}:22`, CLEAN_IMAGE],
      ]);
      if (started.status !== 0) {
        note(`чистый контейнер не запустился: ${started.stderr.slice(0, 200)}`);
        return;
      }
      await until(
        "the clean server to answer",
        async () => await sshAnswers(CLEAN_SSH_PORT),
        60_000,
        500,
      );

      await step(
        "clean-wizard-filled",
        `Второй сервер — «чистый VPS» (контейнер без раздачи): вход по паролю, порт ${CLEAN_SSH_PORT}`,
        async () => {
          await press("Добавить сервер");
          await field("Название", "Чистый VPS");
          await field("Адрес", "127.0.0.1");
          await field("Порт", String(CLEAN_SSH_PORT));
          await field("Домен раздачи", "clean.example.com");
          await chooseX(
            "//label[span[normalize-space()='Вход']]//select",
            "option[@value='password']",
          );
          await field("Пароль", CLEAN_PASSWORD);
        },
      );
      await step(
        "clean-checked",
        "«Дальше» → «Отпечаток верный» → проверка закончилась",
        async () => {
          await press("Дальше");
          await press("Отпечаток верный", "", 60_000);
          await s().findX(
            "//section[contains(@class,'wizard__stage')]//button[normalize-space()='Готово' and not(@disabled)]",
            120_000,
          );
        },
      );
      await step(
        "clean-card",
        "«Готово» — список: у чистого сервера «Раздача не развёрнута»",
        async () => {
          await press("Готово", "//section[contains(@class,'wizard__stage')]");
          await until("the wizard to close", async () => !(await s().has(".wizard")), 15_000);
          await settle();
          await sleep(2500);
        },
      );
      await step(
        "deploy-screen",
        "«Развернуть» у чистого сервера — экран развёртывания (до согласия)",
        async () => {
          await press("Развернуть", "", 30_000);
          await settle(60_000);
          await sleep(4000);
        },
      );
      await step(
        "deploy-ipv6-chosen",
        "Выбран вариант «Оставить IPv6» — состояние кнопки «Согласен, разворачивать»",
        async () => {
          const opt = await s().findX(
            "//label[contains(normalize-space(),'Оставить IPv6')]//input",
            30_000,
          );
          await opt.click();
          await sleep(1500);
        },
      );
      await step(
        "deploy-domain-answered",
        "Ждём ответа проверки домена (до 2 мин); список «Что будет сделано» появляется только при верном домене",
        async () => {
          await until(
            "the domain check",
            async () =>
              !/Спрашиваю у серверов зоны…|Проверяем, куда ведёт домен…/.test(await pageText()) &&
              ((await has("//button[normalize-space()='Согласен, разворачивать']")) ||
                (await has("//button[normalize-space()='Спросить снова']")) ||
                (await has("//button[normalize-space()='Проверить ещё раз']"))),
            120_000,
            1000,
          );
        },
      );
      const canDeploy = await has(
        "//button[normalize-space()='Согласен, разворачивать' and not(@disabled)]",
      );
      note(
        canDeploy
          ? "кнопка «Согласен, разворачивать» доступна; развёртывание не запускалось (на контейнере не дойдёт до конца: домен clean.example.com не ведёт на него)"
          : "кнопки «Согласен, разворачивать» нет вовсе: список «Что будет сделано» и кнопка появляются только когда домен уже ведёт на сервер (clean.example.com на контейнер не ведёт) — развёртывание не запускалось",
      );
      await go("servers");
      await sleep(2000);
      const containerCard =
        "//li[contains(@class,'server')][.//span[@class='server__name' and normalize-space()='Контейнер']]";
      if (await has(`${containerCard}//button[normalize-space()='Сделать активным']`)) {
        await step(
          "make-active-again",
          "«Сделать активным» у «Контейнер» — снова он активный",
          async () => {
            await press("Сделать активным", containerCard);
            await sleep(1500);
          },
        );
      } else {
        note("«Контейнер» остался активным: добавленный второй сервер отметку не забрал (T708)");
      }
      await step("clean-remove-ask", "У «Чистый VPS» нажато «Удалить» — вопрос", async () => {
        await press(
          "Удалить",
          "//li[contains(@class,'server')][.//span[@class='server__name' and normalize-space()='Чистый VPS']]",
        );
      });
      await step("clean-removed", "«Да, удалить» — список", async () => {
        await press("Да, удалить");
        await sleep(1500);
      });
      docker(["rm", "-f", CLEAN_NAME]);
    },
    480_000,
  );

  it.skipIf(!wanted.has("D"))(
    "D — «Video»: three files, the plan, the rungs, «Start all», the first stages",
    async () => {
      const t0 = Date.now();
      beginSection("D");
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      // Nothing left on «Video» from an earlier run, and nothing on the server either: a set of
      // the same film left there would make the first plan say «name taken».
      docker(["exec", NAME, "sh", "-c", "find /var/lib/vrcast/videos -mindepth 1 -delete"]);
      rmSync(FILM1, { force: true });
      ensureFilms();
      for (const v of await invoke<{ id: string }[]>("video_list")) {
        try {
          await invoke("video_remove", { id: v.id });
        } catch {
          // shown as it is
        }
      }
      await go("video");
      await snap("video-empty", "«Видео» при активном сервере, файлов нет");
      const viaButton = await answerDialog([FILM1, FILM2, BROKEN]);
      await step(
        "added-three",
        `«Добавить видео»${viaButton ? "" : " (кнопка недоступна для подмены диалога — через invoke('video_add'))"}: в системном диалоге «выбраны» 3 файла — «Фильм с двумя дорожками.mkv» (1080p, 60 с, eng+rus, rus — вторая), «Короткий 720p.mp4» (20 с), «Битый файл.mp4». Ответ диалога подставлен тестом — WebDriver системное окно не нажмёт`,
        async () => {
          if (viaButton) {
            await press("Добавить видео");
            try {
              await until(
                "cards from the button",
                async () => (await invoke<unknown[]>("video_list")).length >= 2,
                10_000,
              );
              note(`«Добавить видео» нажата, ответ диалога подставлен (${viaButton})`);
            } catch {
              note(
                `подмена ответа диалога (${viaButton}) не сработала — файлы поданы invoke('video_add'), отказ битого на экране не виден`,
              );
              await invoke<Added>("video_add", {
                serverId: await activeServerId(),
                paths: [FILM1, FILM2, BROKEN],
                mediaId: null,
              });
            }
          } else {
            await invoke<Added>("video_add", {
              serverId: await activeServerId(),
              paths: [FILM1, FILM2, BROKEN],
              mediaId: null,
            });
          }
          await sleep(1500);
        },
      );
      const list = await invoke<{ id: string; source_path: string }[]>("video_list");
      const tour: TourState = {
        id1: list.find((a) => a.source_path.endsWith(".mkv"))?.id ?? "",
        id2: list.find((a) => a.source_path.includes("720p"))?.id ?? "",
        caught: [],
        paused: false,
        tasksShot: false,
        secondDone: false,
      };
      saveTour(tour);
      note(`на экране «Видео» карточек: ${list.length} (битый файл — отказ)`);
      if (!tour.id1 || !tour.id2) throw new Error("the two good films were not taken");
      const { id1, id2 } = tour;
      await step("plans-ready", "Ждём планы обеих карточек", async () => {
        await cardUntil(id1, "the first plan", (c) => c.state === "ready", 240_000);
        await cardUntil(id2, "the second plan", (c) => c.state === "ready", 240_000);
      });
      if (await s().has('[data-testid="refused"]')) {
        await step("refused-details", "У отказа битого файла раскрыто «Подробнее»", async () => {
          await press("Подробнее", "//*[@data-testid='refused']", 5_000);
          await sleep(300);
        });
      }
      await step(
        "audio-second",
        "У первой карточки в «Звук» выбрана вторая дорожка (rus) — заранее не выбрана ни одна (T695)",
        async () => {
          const sel = `${inCard(id1)}//label[contains(@class,'video__audio')]//select`;
          // The first option is «Выберите звук» (T695): the track is picked by its value.
          await chooseX(sel, "option[@value='1']");
          await until(
            "the second track to be taken",
            async () => (await (await s().findX(sel)).property("value")) === "1",
            10_000,
          );
        },
      );
      let edited = "";
      /** The window after a rung editor action: alive, or gone blank (then said and reloaded). */
      const editorOutcome = async (what: string): Promise<boolean> => {
        if (await s().has(".video__rungs")) return true;
        const errors = (await caughtErrors()).join(" | ").slice(0, 600);
        note(
          `${what}: ОКНО ОПУСТЕЛО — интерфейс упал (снимок выше). Ошибки страницы: ${errors || "не пойманы"}`,
        );
        await s().execute("location.reload()");
        await s().findFilled(".content");
        await go("video");
        return false;
      };
      const openEditor = async () => {
        await press("Ступени", inCard(id1));
        await s().find(".video__rungs");
        await catchErrors();
      };
      await step("rungs-open", "«Ступени» у первой карточки — редактор открыт", openEditor);
      await step("rungs-idle", "Редактор открыт, ничего не трогаем 4 с", async () => {
        await sleep(4000);
      });
      let alive = await editorOutcome("редактор ступеней сам по себе через 4 с после открытия");
      const attempts: [string, string, () => Promise<void>][] = [
        [
          "rungs-cleared",
          "Поле битрейта нижней ступени очищено (как Backspace до пустого) и введено новое число",
          async () => {
            const inputs = await s().findAll(".video__rungs input[type=number]");
            const last = inputs[inputs.length - 1];
            const was = Number(await last.property("value"));
            const now = was > 1 ? was - 1 : was + 1;
            await last.clear();
            await sleep(300);
            await last.type(String(now));
            edited = `битрейт нижней ступени ${was} → ${now}`;
            await sleep(1000);
          },
        ],
        [
          "rungs-select-type",
          "Значение битрейта нижней ступени заменено выделением (Ctrl+A) и вводом — без пустого поля",
          async () => {
            const inputs = await s().findAll(".video__rungs input[type=number]");
            const last = inputs[inputs.length - 1];
            const was = Number(await last.property("value"));
            const now = was > 1 ? was - 1 : was + 1;
            await last.click();
            await last.type(`\uE009a\uE000${now}`);
            edited = `битрейт нижней ступени ${was} → ${now}`;
            await sleep(1000);
          },
        ],
        [
          "rungs-left-out",
          "Снята галочка «Собирать» у средней ступени (битрейт не трогаем)",
          async () => {
            const boxes = await s().findAll(".video__rungs input[type=checkbox]");
            await boxes[Math.floor(boxes.length / 2)].click();
            edited = "средняя ступень не собирается";
            await sleep(1000);
          },
        ],
      ];
      for (const [name, did, act] of attempts) {
        if (alive) break;
        // The window was reloaded: the editor is opened again for the next way of editing.
        try {
          await openEditor();
        } catch (e) {
          note(`«Ступени» не открылись снова: ${message(e)}`);
          break;
        }
        await step(name, `Окно перезагружено, «Ступени» снова. ${did}`, act);
        alive = await editorOutcome(did);
      }
      if (alive && edited === "") {
        // The editor lived through being left alone: now an edit in it, the ordinary way.
        const [name, did, act] = attempts[0];
        await step(name, did, act);
        alive = await editorOutcome(did);
        for (const [n2, d2, a2] of attempts.slice(1)) {
          if (alive) break;
          await openEditor();
          await step(n2, `Окно перезагружено, «Ступени» снова. ${d2}`, a2);
          alive = await editorOutcome(d2);
        }
      }
      if (!alive) note("ни одной правки ступеней сохранить не удалось — план остался исходным");
      if (await s().has(".video__rungs")) {
        await step(
          "rungs-saved",
          `«Сохранить» в редакторе (${edited}) — план после правки`,
          async () => {
            await press("Сохранить", "//div[contains(@class,'video__rungs')]");
            await cardUntil(id1, "the plan again", (c) => c.state === "ready", 120_000);
            await sleep(500);
          },
        );
      }
      await step("start-all", "«Старт всех»", async () => {
        await press("Старт всех");
        await sleep(1200);
      });
      await watchVideos(tour, t0 + 370_000);
    },
    400_000,
  );

  it.skipIf(!wanted.has("D2"))(
    "D2 — «Video» after a restart: the rest of the stages, the link, a taken name",
    async () => {
      const t0 = Date.now();
      beginSection("D2");
      await ensureServer();
      // Started afresh even when D ran in this same process: this part is «after a restart».
      await shutdown();
      await launch();
      await setLanguage("ru");
      const tour = readTour();
      if (!tour) throw new Error("no state from section D");
      await go("video");
      await snap(
        "after-restart",
        "Приложение запущено заново (новый прогон теста, та же папка данных) — «Видео»",
      );
      await watchVideos(tour, t0 + 250_000);
      const { id1 } = tour;
      if ((await card(id1))?.state === "done") {
        await snap("done", "Первое видео — «Готово», ссылка");
        await step(
          "copied",
          "«Копировать» у ссылки «Авто» (T694: «Авто» и по ссылке на каждое качество)",
          async () => {
            await press("Копировать", inCard(id1));
            await s().findX(`${inCard(id1)}//*[@role='status']`, 5_000);
          },
        );
      } else {
        note(
          `первое видео не дошло до «Готово»: ${JSON.stringify(await card(id1))?.slice(0, 300)}`,
        );
      }
      // The same film once more: its name is taken by the first one's set.
      let id3 = "";
      await step(
        "name-taken-plan",
        "Тот же фильм добавлен ещё раз — план с «Имя занято»",
        async () => {
          if (await answerDialog([FILM1])) await press("Добавить видео");
          else
            await invoke<Added>("video_add", {
              serverId: await activeServerId(),
              paths: [FILM1],
              mediaId: null,
            });
          await sleep(1500);
          const all = await invoke<{ id: string; source_path: string }[]>("video_list");
          id3 = all.filter((v) => v.source_path.endsWith(".mkv") && v.id !== id1).pop()?.id ?? "";
          await cardUntil(id3, "the plan", (c) => c.state === "ready", 120_000);
        },
      );
      if (id3) {
        // T700: the plan already says the name is taken — «Старт» is not pressable, «Другое
        // имя» and «Заменить» are offered at once, and the name is said once.
        await step(
          "name-taken-other-name",
          "«Другое имя» у повторного (Старт недоступен) — поле названия",
          async () => {
            const start = `${inCard(id3)}//button[normalize-space()=${xq("Старт")}]`;
            if (!(await has(`${start}[@disabled]`))) note("«Старт» у повторного доступен");
            await press("Другое имя", inCard(id3));
            await s().findX(`${inCard(id3)}//input`, 5_000);
          },
        );
        await step("name-taken-replace-ask", "«Отмена», затем «Заменить» — вопрос", async () => {
          await press("Отмена", inCard(id3));
          await press("Заменить", inCard(id3));
          await sleep(300);
        });
        await step("name-taken-removed", "«Отмена» и «Убрать» у повторного", async () => {
          await press("Отмена", inCard(id3));
          await press("Убрать", inCard(id3));
          await until("the card to go", async () => !(await card(id3)), 30_000);
        });
      }
    },
    400_000,
  );

  it.skipIf(!wanted.has("E"))(
    "E — the library: the set, a single file, unrecognised, deleting",
    async () => {
      beginSection("E");
      ensureFilms();
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      // What an earlier run of this section made goes first: the medium, the cards, the files.
      {
        const serverId = await activeServerId();
        const lib = await invoke<{ media: { id: string; title: string }[] }>("library_list", {
          serverId,
          refresh: true,
        });
        for (const m of lib.media.filter((x) => x.title === "Одиночный клип")) {
          await invoke("media_delete", { serverId, mediaId: m.id, confirmed: true });
        }
        for (const v of await invoke<{ id: string; title: string; state: string }[]>(
          "video_list",
        )) {
          if (v.state !== "done") await invoke("video_remove", { id: v.id }).catch(() => null);
        }
        docker([
          ...["exec", NAME, "rm", "-f"],
          "/var/lib/vrcast/videos/single-clip.mp4",
          "/var/lib/vrcast/videos/stray-file.mp4",
        ]);
        await invoke("library_list", { serverId, refresh: true });
      }
      await go("library");
      await sleep(1500);
      await snap("library-list", "«Библиотека»: список (свёрнуто)");
      const set =
        "//section[contains(@class,'media')][.//span[contains(@class,'media__title') and contains(normalize-space(),'Фильм с двумя дорожками')]]";
      await step(
        "set-open",
        "Раскрыта карточка фильма с набором: ссылки, файлы ступеней",
        async () => {
          await (await s().findX(`${set}/button[contains(@class,'media__head')]`)).click();
          await sleep(600);
        },
      );
      await step("set-close", "Карточка фильма свёрнута обратно", async () => {
        await (await s().findX(`${set}/button[contains(@class,'media__head')]`)).click();
      });

      // Two files put straight onto the server, the way a person with an SFTP client would.
      for (const f of [SINGLE, STRAY]) {
        const cp = docker(["cp", f, `${NAME}:/var/lib/vrcast/videos/`]);
        if (cp.status !== 0) note(`docker cp ${f}: ${cp.stderr.slice(0, 200)}`);
      }
      const unrec = "//section[contains(@class,'media--unrecognized')]";
      await step(
        "unrecognized",
        "На сервер положены два mp4 мимо приложения (single-clip.mp4, stray-file.mp4); «Обновить»",
        async () => {
          await press("Обновить");
          await s().findX(unrec, 60_000);
          await settle();
        },
      );
      await step("unrecognized-open", "Раскрыта группа «Не распознано»", async () => {
        await (await s().findX(`${unrec}/button`)).click();
        await sleep(1500);
      });
      await step("create-dialog", "«Новое медиа» — диалог, введено «Одиночный клип»", async () => {
        await press("Новое медиа");
        await field("Название", "Одиночный клип");
      });
      await step("created", "«Создать»", async () => {
        await press("Создать");
        await settle();
        await sleep(1000);
      });
      await step(
        "assigned",
        "В «Не распознано» у single-clip.mp4 выбрано «Отнести к медиа» → «Одиночный клип»",
        async () => {
          if (!(await has(`${unrec}//div[contains(@class,'unrecognized__item')]`))) {
            await (await s().findX(`${unrec}/button`)).click();
          }
          const sel = `${unrec}//div[contains(@class,'unrecognized__item')][contains(normalize-space(),'single-clip')]//select`;
          await chooseX(sel, "option[normalize-space()='Одиночный клип']");
          await settle();
          await sleep(1500);
        },
      );
      const single =
        "//section[contains(@class,'media')][.//span[contains(@class,'media__title') and normalize-space()='Одиночный клип']]";
      await step(
        "single-open",
        "Раскрыта карточка «Одиночный клип» — один файл, «Собрать набор»",
        async () => {
          await (await s().findX(`${single}/button[contains(@class,'media__head')]`)).click();
          await sleep(600);
        },
      );
      // «Build a set» opens the system file dialog, which WebDriver cannot press: its answer
      // is put in by the test (answerDialog), the link and the screen are the application's.
      let buildId = "";
      const how = await answerDialog([FILM2]);
      await step(
        "build-set-video",
        `«Собрать набор» у «Одиночный клип»; в системном диалоге «выбран» «Короткий 720p.mp4» (ответ подставлен тестом${how ? "" : " — НЕ удалось, файл подан через invoke"}) — экран «Видео»`,
        async () => {
          const before = new Set((await invoke<{ id: string }[]>("video_list")).map((v) => v.id));
          if (how) {
            await press("Собрать набор", single);
          } else {
            const lib = await invoke<{ media: { id: string; title: string }[] }>("library_list", {
              serverId: await activeServerId(),
              refresh: false,
            });
            const mediaId = lib.media.find((m) => m.title === "Одиночный клип")?.id;
            if (!mediaId) throw new Error("the medium was not found");
            await invoke<Added>("video_add", {
              serverId: await activeServerId(),
              paths: [FILM2],
              mediaId,
            });
            await go("video");
          }
          await until(
            "the new card",
            async () => {
              const now = await invoke<{ id: string }[]>("video_list");
              buildId = now.find((v) => !before.has(v.id))?.id ?? "";
              return buildId;
            },
            15_000,
          );
          await cardUntil(buildId, "the plan", (c) => c.state === "ready", 180_000);
        },
      );
      if (buildId) {
        await step("build-set-started", "«Старт» у этой карточки", async () => {
          await press("Старт", inCard(buildId));
          await cardUntil(buildId, "at work", (c) => c.state === "working", 60_000);
          await sleep(1500);
        });
        await step(
          "build-set-in-library",
          "Пока набор собирается — «Библиотека», карточка «Одиночный клип»",
          async () => {
            await go("library");
            await sleep(1500);
            if (!(await has(`${single}//ul[contains(@class,'file-list')]`))) {
              await (await s().findX(`${single}/button[contains(@class,'media__head')]`)).click();
            }
            await sleep(600);
          },
        );
        await go("video");
        await step("build-set-done", "Набор для «Одиночный клип» — ждём «Готово»", async () => {
          await cardUntil(
            buildId,
            "done",
            (c) => c.state === "done" || c.state === "problem",
            150_000,
          );
        });
      }
      await go("library");
      await sleep(1500);
      await step(
        "delete-ask",
        "У фильма с набором «Удалить медиа» — подтверждение (НЕ подтверждаем)",
        async () => {
          await (await s().findX(`${set}/button[contains(@class,'media__head')]`)).click();
          await sleep(400);
          await press("Удалить медиа", set);
          await s().findX("//button[normalize-space()='Не удалять']", 30_000);
          await sleep(500);
        },
      );
      await step("delete-no", "«Не удалять»", async () => {
        await press("Не удалять");
        await sleep(500);
      });
      await step(
        "file-delete-ask",
        "В «Не распознано» у stray-file.mp4 «Удалить файл» — подтверждение",
        async () => {
          if (!(await has(`${unrec}//div[contains(@class,'unrecognized__item')]`))) {
            await (await s().findX(`${unrec}/button`)).click();
          }
          await press(
            "Удалить файл",
            `${unrec}//div[contains(@class,'unrecognized__item')][contains(normalize-space(),'stray-file')]`,
          );
          await s().findX("//button[normalize-space()='Не удалять']", 30_000);
          await sleep(500);
        },
      );
      await step("file-deleted", "«Удалить» — подтверждено, список", async () => {
        await press("Удалить");
        await settle();
        await sleep(1500);
      });
    },
    480_000,
  );

  it.skipIf(!wanted.has("F"))(
    "F — viewers, one slow, a quality limit and lifting it",
    async () => {
      beginSection("F");
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      const lib = await invoke<{ media: { slug: string; title: string; ladders: unknown[] }[] }>(
        "library_list",
        { serverId: await activeServerId(), refresh: false },
      );
      const film = lib.media.find((m) => m.ladders.length > 0);
      if (!film) {
        note(
          "в библиотеке нет медиа с набором — зрителям нечего смотреть (раздел D не дошёл до конца?)",
        );
        return;
      }
      const ip = serverIp();
      const master = `http://${ip}/videos/${film.slug}/master.m3u8`;
      // Watch the way a player does: the master, a rung's list, then its pieces, round and round.
      const script = (rate: string) =>
        `M=${master}; B=$(dirname $M); while true; do V=$(curl -s $M | grep -v '^#' | grep . | head -1); ` +
        `D=$(dirname $B/$V); curl -s $B/$V > /tmp/v.m3u8; ` +
        `for S in $(grep -v '^#' /tmp/v.m3u8 | grep . | awk '!x[$0]++'); do curl -s ${rate} -o /dev/null $D/$S; done; sleep 1; done`;
      for (const [name, rate] of [
        [VIEWER_FAST, ""],
        [VIEWER_SLOW, "--limit-rate 60k"],
      ] as const) {
        {
          // Always afresh: a viewer left by an earlier run watches whatever it was told then.
          docker(["rm", "-f", name]);
          const r = docker([
            "run",
            "-d",
            "--name",
            name,
            "--entrypoint",
            "sh",
            IMAGE,
            "-c",
            script(rate),
          ]);
          if (r.status !== 0) note(`зритель ${name} не запустился: ${r.stderr.slice(0, 200)}`);
        }
      }
      const slowIp = ipOf(VIEWER_SLOW);
      const fastIp = ipOf(VIEWER_FAST);
      note(
        `зрители — два контейнера с curl по кругу (master → ступень → куски) к ${master}: быстрый ${fastIp}, медленный ${slowIp} (--limit-rate 60k)`,
      );
      await go("viewers");
      await step("viewers-list", "«Зрители» через ~40 с после запуска двух зрителей", async () => {
        await until(
          "two viewers in the list",
          async () => (await s().findAll("[data-testid=viewers-table] tbody tr")).length >= 2,
          120_000,
          1000,
        );
        await sleep(20_000);
      });
      await step("viewers-later", "Ещё через 30 с — скорость/состояние медленного", async () => {
        await sleep(30_000);
      });
      await step(
        "limit-dialog",
        `«Ограничение качества» у медленного зрителя ${slowIp} — предпросмотр`,
        async () => {
          await (await s().find(`[data-testid="limit-${slowIp}"]`)).click();
          await until(
            "the preview",
            async () =>
              !(await s().has('[data-testid="limit-previewing"]')) &&
              (await s().has('[data-testid="confirm"]')),
            60_000,
          );
          await sleep(500);
        },
      );
      const dialog = "//section[@aria-label='Ограничение качества']";
      await step(
        "limit-media-chosen",
        "В «Какое медиа» выбран «Фильм с двумя дорожками» — предпросмотр",
        async () => {
          await chooseX(`${dialog}//select`, "option[normalize-space()='Фильм с двумя дорожками']");
          await sleep(600);
          await until(
            "the preview",
            async () => !(await s().has('[data-testid="limit-previewing"]')),
            60_000,
          );
          await sleep(500);
        },
      );
      await step("limit-cap-2", "Потолок изменён на 2 Мбит/с — предпросмотр", async () => {
        const input = await s().findX(`${dialog}//input`);
        await input.clear();
        await input.type("2");
        await sleep(600);
        await until(
          "the preview",
          async () => !(await s().has('[data-testid="limit-previewing"]')),
          60_000,
        );
        await sleep(500);
      });
      await step("limit-applied", "«Понимаю, ограничить» — итог", async () => {
        await (await s().find('[data-testid="confirm"]')).click();
        await until("the dialog to close", async () => !(await has(dialog)), 90_000, 500);
        await sleep(1000);
      });
      if (await has(dialog)) {
        await step(
          "limit-error-details",
          "Диалог не закрылся — у ошибки раскрыто «Подробнее»",
          async () => {
            await press("Подробнее", dialog, 5_000);
            await sleep(400);
          },
        );
        await step("limit-cancel", "«Отмена» в диалоге ограничения", async () => {
          await press("Отмена", dialog, 5_000);
          await sleep(600);
        });
      }
      await step(
        "limit-viewers-after",
        "Через 30 с после ограничения — список зрителей",
        async () => {
          await sleep(30_000);
        },
      );
      await go("limits");
      await step("limits-list", "«Ограничения» — список действующих", async () => {
        await sleep(2000);
      });
      await step("limit-lifted", "«Снять» у ограничения", async () => {
        await press("Снять", "", 20_000);
        await until(
          "the list to empty",
          async () => await s().has('[data-testid="no-limits"]'),
          60_000,
          500,
        );
      });
    },
    480_000,
  );

  it.skipIf(!wanted.has("G"))(
    "G — diagnostics: the server, the log, why it stalls",
    async () => {
      beginSection("G");
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      await go("diagnostics");
      await step("diag-asking", "«Диагностика» сразу после открытия", async () => {
        await sleep(300);
      });
      await step(
        "diag-result",
        "Диагностика ответила (за 30 минут; зрители из раздела F ещё смотрят)",
        async () => {
          await until(
            "diagnostics to answer",
            async () => !(await s().has('[data-testid="diag-asking"]')),
            180_000,
            500,
          );
        },
      );
      await step("diag-stalls", "Разбор «Почему встаёт картинка» — промотано к нему", async () => {
        await s().execute(
          `const h = [...document.querySelectorAll('h3')].find((x) => /Почему встаёт/.test(x.textContent));
           if (h) h.scrollIntoView();`,
        );
        await sleep(400);
      });
      await step("diag-raw", "Раскрыто «Что именно прочитано»", async () => {
        const raw = await s().findX("//summary[normalize-space()='Что именно прочитано']", 5_000);
        await raw.click();
        await s().execute(`document.querySelector('.content').scrollTop = 0;`);
        await sleep(300);
      });
      await step("diag-10min", "Промежуток 10 минут, «Спросить заново»", async () => {
        await chooseX("//select[@data-testid='diag-period']", "option[@value='10']");
        await sleep(500);
        await until(
          "diagnostics to answer",
          async () => !(await s().has('[data-testid="diag-asking"]')),
          180_000,
          500,
        );
      });
      const how = await answerDialog(FILM1);
      await step(
        "diag-bitrate-peaks",
        `«Пики битрейта файла» → «Выбрать файл»; в системном диалоге «выбран» «Фильм с двумя дорожками.mkv» (ответ подставлен тестом${how ? "" : " — НЕ удалось"}); промотано к результату`,
        async () => {
          if (!how) throw new Error("the dialog's answer could not be put in");
          await press("Выбрать файл");
          await s().find('[data-testid="bitrate-average"]');
          await until(
            "diagnostics to answer again",
            async () => !(await s().has('[data-testid="diag-asking"]')),
            180_000,
            500,
          );
          await s().execute(
            `const h = [...document.querySelectorAll('h3')].find((x) => /Пики битрейта/.test(x.textContent));
             if (h) h.scrollIntoView();`,
          );
          await sleep(400);
        },
      );
      await step(
        "diag-stalls-with-file",
        "Разбор «Почему встаёт картинка» после замера файла (учитывает его пики)",
        async () => {
          await s().execute(
            `const h = [...document.querySelectorAll('h3')].find((x) => /Почему встаёт/.test(x.textContent));
             if (h) h.scrollIntoView();`,
          );
          await sleep(400);
        },
      );
    },
    480_000,
  );

  it.skipIf(!wanted.has("H"))(
    "H — tasks after the work, appearance, about",
    async () => {
      beginSection("H");
      await ensureApp();
      await setLanguage("ru");
      await step("tasks-after", "«Задачи» после работы видео", async () => {
        await go("tasks");
      });
      await step("appearance", "«Оформление» (настройки)", async () => {
        await go("appearance");
      });
      await step(
        "about-from-version",
        "«О программе» — по ссылке «версия …» под названием",
        async () => {
          await (await s().find(".sidebar__version")).click();
          await settle();
        },
      );
      const text = await pageText();
      const build = text.split("\n").find((l) => /^Сборка /.test(l.trim()));
      note(build ? `строка сборки: «${build.trim()}»` : "строки «Сборка …» в «О программе» нет");
      note("окно закрытия с идущей задачей снято в разделе D (снимок leave-confirm)");
    },
    240_000,
  );

  it.skipIf(!wanted.has("I"))(
    "I — the server goes away, and comes back",
    async () => {
      beginSection("I");
      await ensureServer();
      await ensureApp();
      await setLanguage("ru");
      for (const v of [VIEWER_FAST, VIEWER_SLOW]) docker(["stop", "-t", "1", v]);
      const stopped = docker(["stop", "-t", "2", NAME]);
      note(
        `сервер остановлен (docker stop ${NAME}: ${stopped.status === 0 ? "ок" : stopped.stderr.slice(0, 100)})`,
      );
      await step("down-video", "Сервер выключен → «Видео»", async () => {
        await go("video");
        await sleep(3000);
      });
      await step("down-library", "Сервер выключен → «Библиотека»", async () => {
        await go("library");
        await settle(60_000);
        await sleep(3000);
      });
      await step(
        "down-library-refresh",
        "«Обновить» в «Библиотеке» при выключенном сервере",
        async () => {
          await press("Обновить", "", 5_000);
          await settle(60_000);
          await sleep(3000);
        },
      );
      await step("down-viewers", "Сервер выключен → «Зрители» (через 20 с)", async () => {
        await go("viewers");
        await sleep(20_000);
      });
      await step("down-viewers-later", "«Зрители» ещё через 40 с", async () => {
        await sleep(40_000);
      });
      await step("down-servers", "Сервер выключен → «Серверы»", async () => {
        await go("servers");
        await settle(60_000);
        await sleep(3000);
      });
      await step(
        "down-servers-test",
        "«Проверить подключение» при выключенном сервере",
        async () => {
          await press("Проверить подключение");
          await s().findX("//ol[@class='steps']/li", 90_000);
          await s().findX(
            "//button[normalize-space()='Проверить подключение' and not(@disabled)]",
            90_000,
          );
        },
      );
      await step("down-diagnostics", "Сервер выключен → «Диагностика»", async () => {
        await go("diagnostics");
        await until(
          "diagnostics to answer",
          async () => !(await s().has('[data-testid="diag-asking"]')),
          120_000,
          500,
        );
      });
      docker(["start", NAME]);
      await waitServer();
      note("сервер снова запущен (docker start)");
      await step("up-servers", "Сервер снова включён → «Серверы»", async () => {
        await go("servers");
        await settle(60_000);
        await sleep(2000);
      });
      await step("up-library", "Сервер включён → «Библиотека»", async () => {
        await go("library");
        await settle(60_000);
        await sleep(2000);
      });
      await step("up-viewers", "Сервер включён → «Зрители» (через 15 с)", async () => {
        await go("viewers");
        await sleep(15_000);
      });
    },
    480_000,
  );

  it.skipIf(!wanted.has("J"))(
    "J — the key screens in English",
    async () => {
      beginSection("J");
      await ensureServer();
      await ensureApp();
      await step("en-switch", "В боковой панели «Язык» → English; «Серверы»", async () => {
        await chooseX("(//nav[contains(@class,'sidebar')]//select)[1]", "option[@value='en']");
        await waitText(/Servers/);
        await go("servers");
        await sleep(2000);
      });
      await step("en-video", "English: «Video»", async () => {
        await go("video");
      });
      await step("en-library", "English: «Library», фильм раскрыт", async () => {
        await go("library");
        const head =
          "//section[contains(@class,'media')][.//span[contains(@class,'media__title') and contains(normalize-space(),'Фильм с двумя дорожками')]]/button[contains(@class,'media__head')]";
        await (await s().findX(head, 30_000)).click();
        await sleep(600);
      });
      await step("en-diagnostics", "English: «Diagnostics»", async () => {
        await go("diagnostics");
        await until(
          "diagnostics to answer",
          async () => !(await s().has('[data-testid="diag-asking"]')),
          120_000,
          500,
        );
      });
      await setLanguage("ru");
    },
    360_000,
  );

  it.skipIf(!wanted.has("Z"))(
    "Z — tidy away the tour's containers",
    async () => {
      sec = "Z";
      await shutdown();
      removeContainers();
    },
    120_000,
  );
});
