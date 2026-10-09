/**
 * T701 — the sending speed on the «Video» screen: «No limit» or N Mbit/s, one for all, kept.
 *
 * The core is stubbed; what is checked is what a person sees and picks, and that what goes to
 * the core is bytes per second (as every speed in the core), `null` for no limit.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { Settings } from "../../../shared/contract";
import { renderIn, ru } from "../../../test-utils";

let stored: Settings;
const mockSettingsSet = vi.fn<(s: Settings) => Promise<Settings>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      settingsGet: () => Promise.resolve(stored),
      settingsSet: (s: Settings) => mockSettingsSet(s),
    }),
  };
});

const { SendSpeed } = await import("../SendSpeed");
const { SettingsProvider } = await import("../../../app/settings");

function show() {
  return renderIn(
    <SettingsProvider>
      <SendSpeed />
    </SettingsProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  stored = {
    viewer_activity_threshold_s: 30,
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
  mockSettingsSet.mockImplementation((s) => {
    stored = s;
    return Promise.resolve(s);
  });
});

const picked = () =>
  (screen.getByTestId("send-speed") as HTMLSelectElement).selectedOptions[0]?.textContent;

it("starts at «No limit» when nothing was chosen, an older core's answer too", async () => {
  show();
  await waitFor(() => expect(screen.getByTestId("send-speed")).not.toBeDisabled());
  expect(screen.getByText(ru.ui.video.sendSpeed)).toBeInTheDocument();
  expect(picked()).toBe(ru.ui.video.sendNoLimit);
});

it("saves the chosen Mbit/s as bytes per second, and «No limit» as nothing", async () => {
  show();
  const select = await screen.findByTestId("send-speed");
  await waitFor(() => expect(select).not.toBeDisabled());

  fireEvent.change(select, { target: { value: "5" } });
  await waitFor(() =>
    expect(mockSettingsSet).toHaveBeenLastCalledWith(
      expect.objectContaining({ send_limit_bps: 625_000 }),
    ),
  );
  expect(picked()).toBe("5 Мбит/с");

  fireEvent.change(select, { target: { value: "0" } });
  await waitFor(() =>
    expect(mockSettingsSet).toHaveBeenLastCalledWith(
      expect.objectContaining({ send_limit_bps: null }),
    ),
  );
  expect(picked()).toBe(ru.ui.video.sendNoLimit);
});

it("shows what was kept, a value from elsewhere included", async () => {
  stored = { ...stored, send_limit_bps: 3 * 125_000 };
  show();
  await waitFor(() => expect(picked()).toBe("3 Мбит/с"));
});
