/**
 * T342 — starting and stopping the end-to-end harness.
 *
 * Three processes take part and each can fail differently, so each failure says which one it
 * was: `tauri-driver` (the intermediary), the platform's own WebDriver (which it launches),
 * and the application itself.
 *
 * **Nothing here skips.** When a prerequisite is missing the harness fails and says exactly
 * what to run — it replaced a `test:e2e` that printed a note and exited zero, and a check
 * whose green means nothing is worse than no check at all.
 */

import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { Session } from "./webdriver";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = join(HERE, "..", "..");

/** The port `tauri-driver` listens on. Its own default. */
const PORT = 4444;

/** How long to give the intermediary to start answering. */
const START_WITHIN_MS = 20_000;

/**
 * The built application — **the release build, and only that one**.
 *
 * Found on the harness's own first run, and the reason turned out not to be the obvious one.
 * The window opened on the browser's "cannot reach this page": the binary was pointing at the
 * development server (`devUrl`, http://localhost:1420) with nothing there to answer. The
 * harness drove that error page perfectly happily — every check failing for a reason that had
 * nothing to do with the application.
 *
 * **What takes a Tauri binary out of development mode is not the release profile. It is the
 * `custom-protocol` feature**: its own build script reads `dev = !custom_protocol`, so
 * `cargo build --release` alone still produces a binary that wants the dev server. `tauri
 * build` passes the feature itself, and every other way of building has to name it. That is
 * why the feature is declared in `src-tauri/Cargo.toml` and why the command below carries it.
 *
 * So a build that is not the release one is refused rather than driven. Falling back would
 * repeat the mistake of the `test:e2e` this replaced: producing an answer that is not about
 * what was asked.
 */
export function applicationPath(): string {
  const exe = process.platform === "win32" ? "vrcast-studio.exe" : "vrcast-studio";
  // `CARGO_TARGET_DIR` when it is set, and it is on the self-hosted runner: there the build
  // lives outside the checked-out tree so that it survives between runs. Looking only in
  // `src-tauri/target` would send the harness to a directory that is empty by design.
  const targetDir = process.env.CARGO_TARGET_DIR ?? join(APP, "src-tauri", "target");
  const release = join(targetDir, "release", exe);
  if (existsSync(release)) return release;

  const debug = join(targetDir, "debug", exe);
  const note = existsSync(debug)
    ? "\n  There is a debug build, and it will not do: it points the webview at the " +
      "development server rather than at the frontend built into it."
    : "";
  throw new Error(
    `the application is not built for release — ${release} is not there.${note}\n` +
      "  Build it:  npm run build && cargo build --release --features custom-protocol " +
      "--manifest-path src-tauri/Cargo.toml\n" +
      "  (the feature is not optional: without it the window opens on the dev server)",
  );
}

/** Where `tauri-driver` lives. Installed by `cargo install tauri-driver --locked`. */
function driverPath(): string {
  const home =
    process.env.CARGO_HOME ?? join(process.env.USERPROFILE ?? process.env.HOME ?? "", ".cargo");
  const exe = process.platform === "win32" ? "tauri-driver.exe" : "tauri-driver";
  const path = join(home, "bin", exe);
  if (!existsSync(path)) {
    throw new Error(
      `tauri-driver is not installed (looked at ${path}).\n` +
        "  Install it:  cargo install tauri-driver --locked",
    );
  }
  return path;
}

async function answering(): Promise<boolean> {
  try {
    const r = await fetch(`http://127.0.0.1:${PORT}/status`);
    return r.ok;
  } catch {
    return false;
  }
}

export class Harness {
  private constructor(
    private readonly driver: ChildProcess,
    readonly session: Session,
  ) {}

  /**
   * Start everything and open the application.
   *
   * `nativeDriver` is the platform's own WebDriver, found or fetched by
   * `scripts/fetch-webdriver.mjs`.
   */
  static async start(nativeDriver: string): Promise<Harness> {
    const application = applicationPath();
    const said: string[] = [];

    const driver = spawn(driverPath(), ["--port", String(PORT), "--native-driver", nativeDriver], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    // Kept, not printed: on a good run it is noise, and on a bad one it is the only account
    // of what went wrong inside the intermediary.
    driver.stdout?.on("data", (b) => said.push(String(b)));
    driver.stderr?.on("data", (b) => said.push(String(b)));

    const until = Date.now() + START_WITHIN_MS;
    while (!(await answering())) {
      if (driver.exitCode !== null) {
        throw new Error(
          `tauri-driver stopped at once (code ${driver.exitCode}). It said:\n${said.join("")}`,
        );
      }
      if (Date.now() >= until) {
        driver.kill();
        throw new Error(
          `tauri-driver did not start answering on port ${PORT} within ${START_WITHIN_MS / 1000}s. ` +
            `It said:\n${said.join("")}`,
        );
      }
      await new Promise((r) => setTimeout(r, 200));
    }

    try {
      const session = await Session.open(PORT, application);
      return new Harness(driver, session);
    } catch (e) {
      driver.kill();
      throw new Error(
        `the application would not open under the driver (${application}).\n` +
          `  ${String(e)}\n` +
          `  tauri-driver said:\n${said.join("")}`,
      );
    }
  }

  /**
   * Close the session and stop the intermediary.
   *
   * Both, and in this order: a session left open keeps the application's window alive, and on
   * a machine running the harness twice that window is what the second run collides with.
   */
  async stop(): Promise<void> {
    try {
      await this.session.close();
    } finally {
      this.driver.kill();
    }
  }
}
