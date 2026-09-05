/**
 * T330 — a mascot that is turned off is not loaded at all (FR-103).
 *
 * **A file of its own, and that is not tidiness.** A module, once loaded, stays in the
 * registry to the end of the file, so the "was the drawing asked for" counter answers about
 * the first test where the mascot was on rather than about this one. In a shared file such a
 * check passes always — which is what I got, before splitting them apart. Each file has its
 * own registry, and here the counter means exactly what it says.
 *
 * **Checked by the absence of a request, not the absence of a picture.** There is no picture
 * either for a mascot that dutifully downloaded and then hid — and that mascot is precisely
 * what the setting was meant to remove: it gets turned off on a weak machine, and "not
 * visible" does nothing for one.
 */

import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderIn } from "../../../test-utils";
import type { Settings } from "../../../shared/contract";

const shared = vi.hoisted(() => ({ drawingAsked: vi.fn(), settingsGet: vi.fn() }));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      settingsGet: () => shared.settingsGet(),
      settingsSet: async (s: unknown) => s,
    }),
    onTaskProgress: () => Promise.resolve(() => {}),
    onTaskDone: () => Promise.resolve(() => {}),
    onViewersUpdate: () => Promise.resolve(() => {}),
  };
});

vi.mock("../MascotDrawing", async () => {
  shared.drawingAsked();
  return await vi.importActual<typeof import("../MascotDrawing")>("../MascotDrawing");
});

const { Mascot } = await import("../Mascot");
const { SettingsProvider } = await import("../../../app/settings");

const OFF: Settings = {
  viewer_activity_threshold_s: 30,
  geo_refine_outside: false,
  concurrent_heavy_tasks: 1,
  mascot: false,
  animations: true,
  language: null,
  theme: null,
  close_to_tray: true,
  tray_notice_seen: false,
  work_dir: null,
};

describe("a mascot that was turned off", () => {
  it("is never fetched", async () => {
    shared.settingsGet.mockResolvedValue(OFF);
    renderIn(
      <SettingsProvider>
        <Mascot />
      </SettingsProvider>,
    );

    await waitFor(() => expect(shared.settingsGet).toHaveBeenCalled());
    // A little more time: the lazy load would have happened by now if it had begun.
    await new Promise((r) => setTimeout(r, 50));

    expect(shared.drawingAsked).not.toHaveBeenCalled();
    expect(screen.queryByTestId("mascot-slot")).not.toBeInTheDocument();
  });
});
