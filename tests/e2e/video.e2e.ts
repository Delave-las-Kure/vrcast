/**
 * The «Video» screen in the real window — the e2e build, a throwaway server in Docker, a real
 * film, and a person's clicks (acceptance of T672–T679).
 *
 * **Only the e2e build, and it is checked, not assumed.** On Windows an ordinary build opens
 * the person's REAL profiles (the data directory comes from the system, not the environment)
 * and writes secrets into their REAL credential store — that has happened on this machine
 * once. The build with the Cargo feature `e2e` takes its data directory from
 * `VRCAST_DATA_DIR` and keeps secrets in a file there. Before anything is started, the binary
 * is searched for the text only that build carries; a binary without it is not driven at all.
 *
 * Build it (into its own target directory, so the release binary is not touched):
 *
 *   CARGO_TARGET_DIR="F:/Stream Server/.e2e-target" npx tauri build --no-bundle -- --features e2e
 *
 * and point the harness at it with `VRCAST_E2E_TARGET_DIR` (or `VRCAST_E2E_BINARY`).
 * Screenshots go to `VRCAST_E2E_SHOTS` (default: a folder in the system's temporary one).
 *
 * **What is not a click.** The system's file dialog is outside the window, and WebDriver
 * cannot press it: the film is added with `invoke('video_add', …)` from the page — the very
 * call the dialog's answer goes to. The interface language is pinned to Russian through
 * `settings_set` so the buttons can be found by their words. Everything else is pressed.
 *
 * Skipped — said out loud — when the e2e binary, Docker or the test server image is missing,
 * or off Windows (the process handling below is Windows').
 */

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readSync,
  closeSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Harness } from "./session";
import type { Session } from "./webdriver";
import { ensureDriver } from "../../scripts/fetch-webdriver.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = join(HERE, "..", "..");

/** The same image the Rust fixture uses (`src-tauri/tests/integration/fixture.rs`). */
const IMAGE = "vrcast-test-sshd:5";
/** What only the e2e binary carries (`src-tauri/src/store/data_dir.rs`). */
const MARKER = "VRCAST-E2E-BUILD-a7c3f19e";
const KEY = join(APP, "src-tauri", "tests", "fixtures", "encrypted_ed25519.key");
const PASSPHRASE = "test-passphrase-1234";
const FILM_S = 80;

function e2eBinary(): string | null {
  if (process.env.VRCAST_E2E_BINARY) return process.env.VRCAST_E2E_BINARY;
  const dir = process.env.VRCAST_E2E_TARGET_DIR;
  return dir ? join(dir, "release", "vrcast-studio.exe") : null;
}

/** Whether the file carries the e2e build's marker — read through, in blocks. */
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
if (why.length > 0) console.warn(`video.e2e skipped: ${why.join("; ")}`);

// ---------- the server ----------

interface Server {
  name: string;
  ssh: number;
  http: number;
}

function port(name: string, spec: string): number {
  const out = docker(["port", name, spec]).stdout.trim();
  const p = Number(out.split("\n")[0].split(":").pop());
  if (!p) throw new Error(`no port ${spec} for ${name}: ${out}`);
  return p;
}

async function answers(
  p: number,
  first: (c: ReturnType<typeof createConnection>) => void,
  ok: (s: string) => boolean,
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

async function startServer(): Promise<Server> {
  // Built every time, as the Rust fixture does (`fixture.rs::ensure_image`): the tag is shared
  // between worktrees, each with a key of its own, and an image built elsewhere lets nobody
  // in. Cached, it takes a fraction of a second.
  const dir = join(APP, "src-tauri", "tests", "fixtures", "docker");
  const built = docker(["build", "-t", IMAGE, "-f", join(dir, "Dockerfile"), dir]);
  if (built.status !== 0) throw new Error(`the image would not build: ${built.stderr}`);
  const name = `vrcast-e2e-video-${process.pid}`;
  docker(["rm", "-f", name]);
  const run = docker([
    "run",
    "-d",
    "--rm",
    "--name",
    name,
    "-p",
    "127.0.0.1::22",
    "-p",
    "127.0.0.1::80",
    IMAGE,
  ]);
  if (run.status !== 0) throw new Error(`the container would not start: ${run.stderr}`);
  const s = { name, ssh: port(name, "22/tcp"), http: port(name, "80/tcp") };
  const until = Date.now() + 30_000;
  for (;;) {
    const ssh = await answers(
      s.ssh,
      () => {},
      (t) => t.startsWith("SSH-"),
    );
    const http = await answers(
      s.http,
      (c) => c.write("GET / HTTP/1.0\r\nHost: x\r\n\r\n"),
      (t) => t.startsWith("HTTP/"),
    );
    if (ssh && http) return s;
    if (Date.now() > until) throw new Error("the container never answered");
    await new Promise((r) => setTimeout(r, 300));
  }
}

// ---------- the film ----------

function ffmpeg(): string {
  return join(APP, "src-tauri", "binaries", "ffmpeg-x86_64-pc-windows-msvc.exe");
}

function makeFilm(path: string) {
  const dir = dirname(path);
  const srt = join(dir, "film.srt");
  writeFileSync(
    srt,
    "1\n00:00:01,000 --> 00:00:04,000\nHello\n\n2\n00:00:40,000 --> 00:00:44,000\nWorld\n",
  );
  const ch = join(dir, "film.chapters.txt");
  writeFileSync(
    ch,
    `;FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=40000\ntitle=Calm\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=40000\nEND=${FILM_S * 1000}\ntitle=Storm\n`,
  );
  const third = Math.floor(FILM_S / 3);
  const r = spawnSync(
    ffmpeg(),
    [
      ...["-nostdin", "-y", "-v", "error", "-f", "lavfi", "-i"],
      `testsrc2=s=1920x1080:r=30:d=${FILM_S},noise=alls=6:allf=t+u:enable='between(t,${third},${2 * third})',noise=alls=18:allf=t+u:enable='gte(t,${2 * third})'`,
      ...["-f", "lavfi", "-i", `sine=f=440:r=48000:d=${FILM_S}`],
      ...["-f", "lavfi", "-i", `sine=f=1000:r=44100:d=${FILM_S}`],
      ...["-i", srt, "-i", ch],
      ...["-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:s", "-map_chapters", "4"],
      ...[
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
      ],
      ...["-c:a", "aac", "-b:a", "160k", "-c:s", "srt"],
      ...["-metadata:s:a:0", "language=eng", "-metadata:s:a:0", "title=English"],
      ...["-metadata:s:a:1", "language=rus", "-metadata:s:a:1", "title=Russian"],
      path,
    ],
    { encoding: "utf8" },
  );
  if (r.status !== 0) throw new Error(`the film was not made: ${r.stderr}`);
}

// ---------- the application ----------

/** Every process of the e2e build and of its own FFmpeg — and nothing else. */
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

function killOurs(what: "app" | "all") {
  for (const p of ours()) {
    if (what === "app" && !p.path.toLowerCase().endsWith("vrcast-studio.exe")) continue;
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
    await new Promise((r) => setTimeout(r, 200));
  }
}

let scratch: string;
let shots: string;
let server: Server;
let film: string;
let env: Record<string, string>;
let nativeDriver: string;
let harness: Harness | undefined;
let videoId = "";
let shotNo = 0;
const shotList: string[] = [];
const timings: Record<string, number> = {};
const t0 = Date.now();

function s(): Session {
  if (!harness) throw new Error("the application is not running");
  return harness.session;
}

async function open() {
  harness = await Harness.start(nativeDriver, env, binary as string);
  await s().findFilled(".content");
}

async function shot(name: string, css = `[data-testid="video-${videoId}"]`) {
  shotNo += 1;
  const file = join(shots, `${String(shotNo).padStart(2, "0")}-${name}.png`);
  const el = await s().find(css);
  writeFileSync(file, Buffer.from(await el.screenshot(), "base64"));
  shotList.push(file);
}

async function go(route: string) {
  await (await s().find(`a[href="#/${route}"]`)).click();
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

interface Card {
  state: string;
  stage: string | null;
  text: string;
}

async function card(id = videoId): Promise<Card | null> {
  return await s().execute<Card | null>(
    `const el = document.querySelector('[data-testid="video-' + arguments[0] + '"]');
     if (!el) return null;
     const cur = el.querySelector('.video__stage--current');
     return { state: (el.className.match(/video--(\\w+)/) || [])[1], stage: cur ? cur.dataset.stage : null, text: el.innerText };`,
    [id],
  );
}

async function cardUntil(
  what: string,
  ok: (c: Card) => boolean,
  ms: number,
  id = videoId,
): Promise<Card> {
  const until = Date.now() + ms;
  let last: Card | null = null;
  for (;;) {
    last = await card(id);
    if (last && ok(last)) return last;
    if (Date.now() > until)
      throw new Error(`${what}: not within ${ms / 1000}s; the card: ${JSON.stringify(last)}`);
    await new Promise((r) => setTimeout(r, 40));
  }
}

/** Remember every stage the card's bar shows, from inside the page: a stage that lasts
 *  70 ms is shown to a person and gone before the next poll. */
async function watchStages() {
  await s().execute(`
    if (window.__watching) return;
    window.__watching = true;
    window.__seen = [];
    const look = () => document.querySelectorAll('[data-testid^="video-"]').forEach((el) => {
      const cur = el.querySelector('.video__stage--current');
      const st = (el.className.match(/video--(\\w+)/) || [])[1];
      const key = (cur ? cur.dataset.stage : '-') + '/' + st;
      window.__last = window.__last || {};
      if (window.__last[el.dataset.testid] !== key) {
        window.__last[el.dataset.testid] = key;
        window.__seen.push({ id: el.dataset.testid, key, t: Date.now() });
      }
    });
    new MutationObserver(look).observe(document.body, { subtree: true, childList: true, attributes: true });
    look();`);
}

async function seenStages(): Promise<{ id: string; key: string; t: number }[]> {
  return await s().execute(`return window.__seen || [];`);
}

const seenAll: { id: string; key: string; t: number }[] = [];

function button(id: string, label: string) {
  return `//li[@data-testid='video-${id}']//button[normalize-space()='${label}']`;
}

describe.skipIf(why.length > 0)("the «Video» screen, in the real window", () => {
  beforeAll(async () => {
    const base = process.env.VRCAST_E2E_SCRATCH ?? tmpdir();
    mkdirSync(base, { recursive: true });
    scratch = mkdtempSync(join(base, "vrcast-video-e2e-"));
    shots = process.env.VRCAST_E2E_SHOTS ?? join(tmpdir(), "vrcast-e2e-shots");
    mkdirSync(shots, { recursive: true });
    server = await startServer();
    film = join(scratch, "films", "Window Film.mkv");
    mkdirSync(dirname(film), { recursive: true });
    makeFilm(film);
    const data = join(scratch, "data");
    mkdirSync(join(scratch, "tmp"), { recursive: true });
    env = {
      VRCAST_DATA_DIR: data,
      VRCAST_E2E_VERIFY_ORIGIN: `http://127.0.0.1:${server.http}`,
      TMP: join(scratch, "tmp"),
      TEMP: join(scratch, "tmp"),
    };
    nativeDriver = await ensureDriver();
    await open();
    // The catch that has to hold before anything is pressed: the application writes where it
    // was told, not into the person's profile.
    expect(existsSync(join(data, "vrcast-studio.sqlite"))).toBe(true);
    const settings = await invoke<Record<string, unknown>>("settings_get");
    await invoke("settings_set", { settings: { ...settings, language: "ru" } });
    await s().execute("location.reload()");
    await s().findFilled(".content");
  }, 180_000);

  afterAll(async () => {
    writeFileSync(
      join(shots, "summary.json"),
      JSON.stringify({ shots: shotList, timings, seen: seenAll, scratch }, null, 2),
    );
    try {
      await harness?.stop();
    } catch {
      // the window may be gone already
    }
    killOurs("all");
    if (server) docker(["rm", "-f", server.name]);
  }, 60_000);

  it("adds the server through the wizard", async () => {
    await go("servers");
    await (await s().findX("//button[normalize-space()='Добавить сервер']")).click();
    const field = async (label: string, value: string) => {
      const input = await s().findX(`//label[span[normalize-space()='${label}']]//input`);
      await input.clear();
      await input.type(value);
    };
    await field("Название", "Container");
    await field("Адрес", "127.0.0.1");
    await field("Порт", String(server.ssh));
    await field("Домен раздачи", "stream.example.com");
    await field("Путь к приватному ключу", KEY);
    await field("Парольная фраза ключа", PASSPHRASE);
    await shot("wizard-filled", ".wizard");
    await (await s().findX("//button[normalize-space()='Дальше']")).click();
    await (await s().findX("//button[normalize-space()='Отпечаток верный']", 30_000)).click();
    // The check's own list (class exactly `steps`, not the header's `wizard__steps`), and its
    // buttons enabled again — the check has finished.
    await s().findX("//ol[@class='steps']/li", 60_000);
    const finish = await s().findX(
      "//section[contains(@class,'wizard__stage')]//button[normalize-space()='Готово' and not(@disabled)]",
      90_000,
    );
    await shot("wizard-checked", ".wizard");
    await finish.click();
    const until = Date.now() + 15_000;
    while (await s().has(".wizard")) {
      if (Date.now() > until) throw new Error("the wizard did not close");
      await new Promise((r) => setTimeout(r, 200));
    }
    const list = await s().findFilled(".content");
    expect(await list.text()).toContain("Container");
  }, 120_000);

  it("shows the plan, takes the second audio track by a click, and starts", async () => {
    await go("video");
    await s().findX("//h1[normalize-space()='Видео']");
    const servers = await invoke<{ id: string }[]>("servers_list");
    const added = await invoke<{ added: { id: string }[]; refused: unknown[] }>("video_add", {
      serverId: servers[0].id,
      paths: [film],
      mediaId: null,
    });
    expect(added.refused).toEqual([]);
    videoId = added.added[0].id;
    await watchStages();
    await cardUntil("the plan", (c) => c.state === "ready", 60_000);
    await s().find(`[data-testid="video-${videoId}"] [data-testid="plan"]`);
    // The second track, chosen the way a person chooses it: the list, then the option.
    const select = await s().find(`[data-testid="video-${videoId}"] .video__audio select`);
    await select.click();
    await (
      await s().find(`[data-testid="video-${videoId}"] .video__audio select option[value="1"]`)
    ).click();
    const until = Date.now() + 10_000;
    while ((await select.property("value")) !== "1") {
      if (Date.now() > until) throw new Error("the second track was not taken");
      await new Promise((r) => setTimeout(r, 100));
    }
    await shot("plan");
    timings.start = Date.now() - t0;
    await (await s().findX(button(videoId, "Старт"))).click();
  }, 120_000);

  it("measures, encodes, pauses and goes on by the buttons, and carries on by itself after a restart", async () => {
    await cardUntil("measuring", (c) => c.stage === "measuring" && c.state === "working", 60_000);
    timings.measuring = Date.now() - t0;
    await shot("measuring");
    await cardUntil("encoding", (c) => c.stage === "encoding" && c.state === "working", 240_000);
    timings.encoding = Date.now() - t0;
    await shot("encoding");

    await (await s().findX(button(videoId, "Пауза"))).click();
    await cardUntil("paused", (c) => c.state === "paused", 15_000);
    await shot("paused");
    await (await s().findX(button(videoId, "Продолжить"))).click();
    const going = await cardUntil("going again", (c) => c.state === "working", 15_000);
    await shot(`resumed-${going.stage}`);
    seenAll.push(...(await seenStages()));

    // The application stops as if the machine had — no goodbye — and is started again with
    // the same data directory. Nobody presses anything after it.
    const before = await card();
    timings.killed = Date.now() - t0;
    killOurs("app");
    try {
      await harness?.stop();
    } catch {
      // the window is already gone
    }
    harness = undefined;
    await driverDown();
    await open();
    timings.reopened = Date.now() - t0;
    await go("video");
    await watchStages();
    const after = await cardUntil(
      "carried on",
      (c) => c.state === "working" && c.stage !== null,
      60_000,
    );
    await shot(`after-restart-${after.stage}`);
    expect(before?.state).toBe("working");
    expect(["encoding", "uploading", "cutting", "verifying"]).toContain(after.stage);
  }, 420_000);

  it("shows every stage on the way to the link, and the link copies", async () => {
    const caught = new Set<string>();
    const until = Date.now() + 300_000;
    for (;;) {
      const c = await card();
      if (c?.state === "done") break;
      if (c?.state === "problem") throw new Error(`stopped on a problem: ${c.text}`);
      if (
        c?.stage &&
        !caught.has(c.stage) &&
        ["uploading", "cutting", "verifying"].includes(c.stage)
      ) {
        caught.add(c.stage);
        timings[c.stage] = Date.now() - t0;
        await shot(c.stage);
      }
      if (Date.now() > until) throw new Error(`not done: ${JSON.stringify(c)}`);
      await new Promise((r) => setTimeout(r, 30));
    }
    timings.done = Date.now() - t0;
    seenAll.push(...(await seenStages()));
    await shot("done");
    const shown = new Set(
      seenAll.filter((x) => x.id === `video-${videoId}`).map((x) => x.key.split("/")[0]),
    );
    for (const stage of ["measuring", "encoding", "uploading", "cutting", "verifying"]) {
      expect(shown, `the bar never showed ${stage}: ${JSON.stringify(seenAll)}`).toContain(stage);
    }

    const link = await s().find(`[data-testid="video-${videoId}"] .video__link a`);
    const href = (await link.attribute("href")) ?? "";
    expect(href).toMatch(/^https:\/\/stream\.example\.com\/videos\/window-film\/master\.m3u8$/);
    // The link's path, asked of the container the domain stands for here: a working master.
    const master = await fetch(`http://127.0.0.1:${server.http}${new URL(href).pathname}`);
    expect(master.status).toBe(200);
    expect(await master.text()).toContain("#EXT-X-STREAM-INF");

    await (await s().findX(button(videoId, "Копировать"))).click();
    await s().findX(`//li[@data-testid='video-${videoId}']//span[@role='status']`);
    const said = await (
      await s().find(`[data-testid="video-${videoId}"] .video__link [role="status"]`)
    ).text();
    await shot("copied");
    expect(said).toBe("Скопировано");
    const clip = spawnSync("powershell", ["-NoProfile", "-Command", "Get-Clipboard"], {
      encoding: "utf8",
    });
    expect(clip.stdout.trim()).toBe(href);
  }, 360_000);

  it("the library shows the medium with its set and the set's rung files", async () => {
    await go("library");
    const head = await s().findX(
      "//section[contains(@class,'media')]//button[contains(@class,'media__head')][.//span[normalize-space()='Window Film']]",
      60_000,
    );
    await head.click();
    const files = await s().find('[data-testid^="set-files-"]');
    const text = await files.text();
    expect(text).toContain("Файлы ступеней набора");
    expect(text).toMatch(/window-film_\d+\.mp4/);
    await s().find('[data-testid^="ladder-sets-"]');
    await shot("library", "section.media");
  }, 120_000);

  it("a taken name stops the second video on one line with its buttons", async () => {
    await go("video");
    const servers = await invoke<{ id: string }[]>("servers_list");
    const again = await invoke<{ added: { id: string }[] }>("video_add", {
      serverId: servers[0].id,
      paths: [film],
      mediaId: null,
    });
    const second = again.added[0].id;
    const ready = await cardUntil("the second plan", (c) => c.state === "ready", 60_000, second);
    expect(ready.text).toContain("занято");
    const prevId = videoId;
    videoId = second;
    await shot("name-taken-plan");
    await (await s().findX(button(second, "Старт"))).click();
    const stopped = await cardUntil("the problem", (c) => c.state === "problem", 60_000, second);
    await shot("name-taken-problem");
    const lines = await s().findAll(`[data-testid="video-${second}"] .video__problem-line`);
    expect(lines.length).toBe(1);
    await s().findX(button(second, "Заменить"));
    await s().findX(button(second, "Повторить"));
    expect(stopped.text).not.toContain("Пауза");
    videoId = prevId;
  }, 120_000);

  it("leaves no FFmpeg of its own running and nothing in the person's profile", () => {
    const left = ours().filter((p) => !p.path.toLowerCase().endsWith("vrcast-studio.exe"));
    expect(left).toEqual([]);
    // The secrets went to the file in the data directory, not to the system's store.
    expect(existsSync(join(env.VRCAST_DATA_DIR, "e2e-secrets.json"))).toBe(true);
    console.log(`video.e2e: shots in ${shots}; timings ${JSON.stringify(timings)}`);
  });
});
