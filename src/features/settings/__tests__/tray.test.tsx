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
let saved: Settings | null = null;

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
      settingsGet: () => Promise.resolve(settings),
      settingsSet: (s: Settings) => {
        saved = s;
        return Promise.resolve(s);
      },
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

// ⚠ **The settings come from the real provider, not from a mock of the module** (T494).
// Replacing `app/settings` was this file's own invention — twenty other test files reach the
// same end by stubbing `settingsGet`/`settingsSet` and rendering inside the real
// `SettingsProvider`. The invention cost something: with the module mocked, the diagnosis
// screen's eight tests failed once in six runs with "useSettings is not a function", in a
// file that mocks nothing of the kind. An intermittent failure in somebody else's tests is
// the worst way to pay for a shortcut in your own.
const { CloseButton } = await import("../CloseButton");
const { SettingsProvider } = await import("../../../app/settings");
const { useTrayNotice } = await import("../../tasks/notifications");

function Listener() {
  useTrayNotice();
  return null;
}

/** The section as a person meets it: inside the settings the application really keeps. */
function show() {
  return renderIn(
    <SettingsProvider>
      <CloseButton />
    </SettingsProvider>,
  );
}

beforeEach(() => {
  sendNotification.mockReset();
  saved = null;
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
  show();
  const box = await screen.findByTestId("close-to-tray-switch");
  expect((box as HTMLInputElement).checked).toBe(true);
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeHides);

  // What a click is worth is what reaches the store, not what a spy saw: the switch is bound
  // to the settings the application actually keeps.
  fireEvent.click(box);
  await waitFor(() => expect(saved?.close_to_tray).toBe(false));
});

it("says the other thing when the other thing was asked for", async () => {
  settings = { ...settings, close_to_tray: false };
  show();
  await screen.findByTestId("close-to-tray-switch");
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeExits);
});

it("offers no choice where there is nowhere to hide", async () => {
  // The setting cannot ask for the window to be lost. Showing a switch that the core
  // overrules would be worse than showing none: it would say the decision was theirs.
  trayState = "unavailable";
  show();
  await screen.findByTestId("close-behaviour");
  expect(screen.queryByTestId("close-to-tray-switch")).toBeNull();
  expect(screen.getByTestId("close-behaviour").textContent).toBe(ru.ui.appearance.closeExits);
});

it("says it does not know rather than guessing", async () => {
  trayState = "unknown" as TrayState;
  show();
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
