/**
 * T546 — how many heavy tasks may run at once.
 *
 * The core is stubbed; what is checked here is what a person can see and press, and that the
 * screen shows what the core actually kept rather than what was typed (the backend clamps
 * `concurrent_heavy_tasks` against `MAX_HEAVY_TASKS`, a number this side of the contract never
 * sees at all — see `HeavyTasks.tsx`'s own doc comment).
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { Settings } from "../../../shared/contract";
import { renderIn } from "../../../test-utils";

let stored: Settings;
const mockSettingsSet = vi.fn<(s: Settings) => Promise<Settings>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      settingsGet: () => Promise.resolve(stored),
      settingsSet: (s: Settings) => mockSettingsSet(s),
    }),
  };
});

const { HeavyTasks } = await import("../HeavyTasks");
const { SettingsProvider } = await import("../../../app/settings");

function show() {
  return renderIn(
    <SettingsProvider>
      <HeavyTasks />
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

it("shows what the core answered, not a silent default", async () => {
  stored = { ...stored, concurrent_heavy_tasks: 3 };
  show();
  await waitFor(() =>
    expect((screen.getByTestId("heavy-tasks-input") as HTMLInputElement).value).toBe("3"),
  );
});

it("saves a whole settings object with the new number in it", async () => {
  show();
  const input = await screen.findByTestId("heavy-tasks-input");
  fireEvent.change(input, { target: { value: "4" } });

  await waitFor(() =>
    expect(mockSettingsSet).toHaveBeenCalledWith(
      expect.objectContaining({ concurrent_heavy_tasks: 4 }),
    ),
  );
});

it("does not let a value below one reach the core", async () => {
  show();
  const input = await screen.findByTestId("heavy-tasks-input");
  fireEvent.change(input, { target: { value: "0" } });

  await waitFor(() =>
    expect(mockSettingsSet).toHaveBeenCalledWith(
      expect.objectContaining({ concurrent_heavy_tasks: 1 }),
    ),
  );

  fireEvent.change(input, { target: { value: "-5" } });
  await waitFor(() =>
    expect(mockSettingsSet).toHaveBeenLastCalledWith(
      expect.objectContaining({ concurrent_heavy_tasks: 1 }),
    ),
  );
});

it("shows the ceiling the core actually clamped to, rather than what was typed", async () => {
  // The ceiling (`MAX_HEAVY_TASKS`) is not known here at all — this is not a test of "8" as a
  // number, it is a test that the screen trusts the answer over the input.
  mockSettingsSet.mockImplementation(async (s) => {
    const clamped = { ...s, concurrent_heavy_tasks: 8 };
    stored = clamped;
    return clamped;
  });
  show();
  const input = await screen.findByTestId("heavy-tasks-input");
  fireEvent.change(input, { target: { value: "20" } });

  await waitFor(() =>
    expect((screen.getByTestId("heavy-tasks-input") as HTMLInputElement).value).toBe("8"),
  );
});
