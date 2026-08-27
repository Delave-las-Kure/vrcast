/**
 * T342 — the browser driver the end-to-end harness needs, fetched rather than assumed.
 *
 * `tauri-driver` does not talk to the webview itself: it forwards to the platform's own
 * WebDriver, and that driver has to be **the same version as the webview**. On Windows that
 * is Microsoft's `msedgedriver`, whose version has to match the installed Edge; on Linux it
 * is `WebKitWebDriver`, which comes from the system's own package.
 *
 * The two are not symmetrical and this script does not pretend they are: on Windows it can
 * fetch the right one, on Linux it can only say which package to install. Downloading a
 * system library behind the package manager's back is how a machine ends up with two of
 * them.
 *
 * **What it must never do is succeed quietly when it did nothing.** The harness this feeds
 * replaced a `test:e2e` that printed a note and exited zero, and a check that always passes
 * is worse than no check: its green is read as a result.
 */

import { createWriteStream, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = join(HERE, "..");

/**
 * Where the driver goes.
 *
 * Under `target/`, which is already ignored by git: it is twenty megabytes of somebody
 * else's binary, and it belongs in the repository exactly as little as FFmpeg does.
 */
export const DRIVER_DIR = join(APP, "src-tauri", "target", "webdriver");

/** The registry key Edge keeps its version under. */
const EDGE_KEY =
  "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{56EB18F8-B008-4CBD-B6D2-8C97FE7E9062}";

function edgeVersion() {
  const out = spawnSync("reg", ["query", EDGE_KEY, "/v", "pv"], { encoding: "utf8" });
  const found = /pv\s+REG_SZ\s+([0-9.]+)/.exec(out.stdout ?? "");
  return found ? found[1] : null;
}

async function fetchWindowsDriver() {
  const version = edgeVersion();
  if (!version) {
    throw new Error(
      "Microsoft Edge was not found in the registry. The end-to-end harness drives the " +
        "application's webview, and on Windows that webview is Edge's — without it there is " +
        "nothing to drive.",
    );
  }

  const exe = join(DRIVER_DIR, "msedgedriver.exe");
  const stamp = join(DRIVER_DIR, "version.txt");
  if (existsSync(exe) && existsSync(stamp) && readFileSync(stamp, "utf8").trim() === version) {
    console.log(`Edge WebDriver ${version} is already in place: ${exe}`);
    return exe;
  }

  // The exact version, not "latest". A driver a minor version away from the browser refuses
  // to start with a message about the mismatch, and that message reaches the harness as a
  // failure nobody can read.
  const url = `https://msedgedriver.microsoft.com/${version}/edgedriver_win64.zip`;
  console.log(`Downloading Edge WebDriver ${version}…`);

  mkdirSync(DRIVER_DIR, { recursive: true });
  const zip = join(DRIVER_DIR, "edgedriver.zip");
  const answer = await fetch(url);
  if (!answer.ok) {
    throw new Error(
      `Edge WebDriver ${version} is not published (${answer.status} at ${url}). Edge updates ` +
        "itself and its driver follows a moment later; try again, or install the driver by hand.",
    );
  }
  await pipeline(Readable.fromWeb(answer.body), createWriteStream(zip));

  // Expand-Archive rather than a zip library: unpacking one archive is not worth a
  // dependency, and every dependency here has to pass the licence check and be listed.
  execFileSync(
    "powershell",
    [
      "-NoProfile",
      "-Command",
      `Expand-Archive -Force -LiteralPath '${zip}' -DestinationPath '${DRIVER_DIR}'`,
    ],
    { stdio: "inherit" },
  );
  rmSync(zip, { force: true });

  if (!existsSync(exe)) {
    throw new Error(`the archive unpacked but ${exe} is not there`);
  }
  const { writeFileSync } = await import("node:fs");
  writeFileSync(stamp, version, "utf8");
  console.log(`Edge WebDriver ${version} is ready: ${exe}`);
  return exe;
}

function findLinuxDriver() {
  const out = spawnSync("which", ["WebKitWebDriver"], { encoding: "utf8" });
  const path = (out.stdout ?? "").trim();
  if (path) {
    console.log(`WebKitWebDriver is in place: ${path}`);
    return path;
  }
  throw new Error(
    "WebKitWebDriver was not found. It comes from the system's own package, and fetching it " +
      "behind the package manager's back would leave the machine with two of them.\n" +
      "  Install it:  sudo apt-get install -y webkit2gtk-driver\n" +
      "  A window also needs somewhere to open — on a machine with no display, run the " +
      "harness under xvfb-run.",
  );
}

/** Where the native driver is, fetching it first if that is possible on this platform. */
export async function ensureDriver() {
  return process.platform === "win32" ? await fetchWindowsDriver() : findLinuxDriver();
}

if (
  import.meta.url === `file://${process.argv[1]}` ||
  process.argv[1]?.endsWith("fetch-webdriver.mjs")
) {
  ensureDriver().catch((e) => {
    console.error(String(e.message ?? e));
    process.exit(1);
  });
}
