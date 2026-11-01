/** Microsoft's explicit WebView2 attach protocol for the clean hosted Windows runner. */
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface AttachedApplication {
  debuggerAddress: string;
  stop(): Promise<void>;
}

async function unusedLoopbackPort(): Promise<number> {
  const reservation = createServer();
  await new Promise<void>((resolve, reject) => {
    reservation.once("error", reject);
    reservation.listen(0, "127.0.0.1", resolve);
  });
  const address = reservation.address();
  if (!address || typeof address === "string") throw new Error("no loopback port assigned");
  await new Promise<void>((resolve, reject) =>
    reservation.close((e) => (e ? reject(e) : resolve())),
  );
  return address.port;
}

export async function startAttachedApplication(
  application: string,
  extraEnv: Record<string, string>,
  said: string[],
  patienceMs: number,
): Promise<AttachedApplication> {
  const port = await unusedLoopbackPort();
  const debuggerAddress = `127.0.0.1:${port}`;
  const profile = mkdtempSync(join(tmpdir(), "vrcast-webview-"));
  let child: ChildProcess | undefined;
  const stop = async () => {
    if (child?.pid && child.exitCode === null) {
      // Only our own child, never somebody else's app or a process-name wildcard.
      spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    }
    for (let attempt = 0; ; attempt++) {
      try {
        rmSync(profile, { recursive: true, force: true });
        break;
      } catch (e) {
        if (attempt === 4) throw e;
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    }
  };
  try {
    // No DevToolsActivePort discovery: the writable profile and endpoint belong to this
    // harness. Driver version matching remains ensureDriver's job and is not bypassed.
    child = spawn(application, [], {
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        ...extraEnv,
        TAURI_WEBVIEW_AUTOMATION: "true",
        WEBVIEW2_USER_DATA_FOLDER: profile,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      },
    });
    child.stdout?.on("data", (b) => said.push(String(b)));
    child.stderr?.on("data", (b) => said.push(String(b)));
    let startError: Error | undefined;
    child.on("error", (e) => {
      startError = e;
    });
    const until = Date.now() + patienceMs;
    for (;;) {
      if (startError) throw startError;
      if (child.exitCode !== null) throw new Error(`application exited ${child.exitCode}`);
      try {
        const response = await fetch(`http://${debuggerAddress}/json/version`, {
          signal: AbortSignal.timeout(1000),
        });
        const version = response.ok
          ? ((await response.json()) as { webSocketDebuggerUrl?: string })
          : undefined;
        if (version?.webSocketDebuggerUrl) return { debuggerAddress, stop };
      } catch {
        /* the webview has not started listening yet */
      }
      if (Date.now() >= until)
        throw new Error("the application did not expose its WebView2 debugging endpoint");
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  } catch (e) {
    await stop();
    throw e;
  }
}
