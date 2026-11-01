import { afterEach, expect, it, vi } from "vitest";
import { Session } from "./webdriver";

afterEach(() => vi.unstubAllGlobals());

it("attaches to an explicitly launched WebView2 without asking the driver to launch another app", async () => {
  const request = vi.fn().mockResolvedValue({
    ok: true,
    text: async () => JSON.stringify({ value: { sessionId: "owned-webview" } }),
  });
  vi.stubGlobal("fetch", request);
  const session = await Session.open(4444, "C:/isolated/app.exe", "127.0.0.1:9222");
  expect(session.id).toBe("owned-webview");
  const body = JSON.parse(request.mock.calls[0][1].body as string);
  expect(body.capabilities.alwaysMatch).toEqual({
    browserName: "webview2",
    "ms:edgeOptions": { debuggerAddress: "127.0.0.1:9222" },
  });
});

it("keeps the intermediary launch protocol for ordinary sessions", async () => {
  const request = vi.fn().mockResolvedValue({
    ok: true,
    text: async () => JSON.stringify({ value: { sessionId: "launched-webview" } }),
  });
  vi.stubGlobal("fetch", request);
  await Session.open(4444, "C:/isolated/app.exe");
  const body = JSON.parse(request.mock.calls[0][1].body as string);
  expect(body.capabilities.alwaysMatch).toEqual({
    "tauri:options": { application: "C:/isolated/app.exe" },
  });
});
