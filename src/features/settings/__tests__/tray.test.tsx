/**
 * T399 — the close button as a choice, and being told where the window went.
 *
 * ⚠ **The check this stands in for cannot be made.** Whether the tray icon is visible has no
 * answer: `rect()` on Linux is always `None`, tray events are unsupported there, and nothing
 * reports a failure (R-35). On Windows 11 a new icon goes into the overflow, invisible in a
 * different way. So the window may vanish into nothing a person can see. What can be checked
 * is that the application says where it went, once, and that the button obeys what was asked.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { Settings, TrayState } from "../../../shared/contract";
import { renderIn, ru } from "../../../test-utils";

const sendNotification = vi.fn();
const notify = vi.fn<() => Promise<void>>();
let trayState: TrayState = "installed";
let hidden: (() => void) | null = null;
let settings: Settings;
const update = vi.fn<(patch: Partial<Settings>) => void>();

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: () => Promise.resolve(true),
  requestPermission: () => Promise.resolve("granted"),
  sendNotification: (...args: unknown[]) => sendNotification(...args),
}));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      trayState: () => Promise.resolve(trayState),
    }),
    onHiddenToTray: async (handler: () => void) => {
      hidden = handler;
      void notify();
      return () => {
        if (hidden === handler) hidden = null;
      };
    },
  };
});

vi.mock("../../../app/settings", async () => {
  const actual =
    await vi.importActual<typeof import("../../../app/settings")>("../../../app/settings");
  return {
    ...actual,
    useSettings: () => ({ settings, update, error: null }),
  };
});

const { CloseButton } = await import("../CloseButton");
const { useTrayNotice } = await import("../../tasks/notifications");

function Listener() {
  useTrayNotice();
  return null;
}

beforeEach(() => {
  sendNotification.mockReset();
  update.mockReset();
  hidden = null;
  trayState = "installed";
  settings = {
    viewer_activity_threshold_s: 120,
    geo_refine_outside: false,
    concurrent_heavy_tasks: 1,
    mascot: true,
    animations: true,
    language: null,
    theme: null,
    close_to_tray: true,
    tray_notice_seen: false,
    work_dir: null,
  };
});

// ---------- the choice ----------

it("offers the choice where there is somewhere to hide, and says what will happen", async () => {
  renderIn(<CloseButton />);
  const box = await screen.findByTestId("close-to-tray-switch");
  expect((box as HTMLInputElement).checked).toBe(true);
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeHides);

  fireEvent.click(box);
  expect(update).toHaveBeenCalledWith({ close_to_tray: false });
});

it("says the other thing when the other thing was asked for", async () => {
  settings = { ...settings, close_to_tray: false };
  renderIn(<CloseButton />);
  await screen.findByTestId("close-to-tray-switch");
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeExits);
});

it("offers no choice where there is nowhere to hide", async () => {
  // The setting cannot ask for the window to be lost. Showing a switch that the core
  // overrules would be worse than showing none: it would say the decision was theirs.
  trayState = "unavailable";
  renderIn(<CloseButton />);
  await screen.findByTestId("close-behaviour");
  expect(screen.queryByTestId("close-to-tray-switch")).toBeNull();
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeExits);
});

it("says it does not know rather than guessing", async () => {
  trayState = "unknown" as TrayState;
  renderIn(<CloseButton />);
  await screen.findByTestId("close-behaviour");
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeUnknown);
  expect(screen.queryByTestId("close-to-tray-switch")).toBeNull();
});

// ---------- being told where it went ----------

it("says where the window went, in words a person can act on", async () => {
  renderIn(<Listener />);
  await waitFor(() => expect(hidden).not.toBeNull());
  expect(sendNotification).not.toHaveBeenCalled();

  hidden?.();
  await waitFor(() => expect(sendNotification).toHaveBeenCalledTimes(1));
  const said = sendNotification.mock.calls[0][0] as { title: string; body: string };
  expect(said.title).toBe(ru.ui.notifications.hiddenTitle);
  expect(said.body).toBe(ru.ui.notifications.hiddenBody);
  // The body has to carry the way back. A notice saying only "the window is hidden" leaves
  // somebody looking at a taskbar with nothing on it.
  expect(said.body).toContain(ru.ui.tray.show);
});
