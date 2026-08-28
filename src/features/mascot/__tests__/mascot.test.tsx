/**
 * T330 — the mascot, from a person's side and from the machine's.
 *
 * Three promises, and the second is the only thing that makes the "turn it off" setting mean
 * anything at all:
 *
 * 1. **the mood comes from real task events** (FR-102) rather than from a source of its own:
 *    let it drift from the task list and the mascot waves at a task that has just failed;
 * 2. **turned off means not loaded at all** (FR-103). Checked by the **absence of a request**
 *    rather than the absence of a picture: there is no picture either when one simply did not
 *    render, so a picture check would pass on a mascot that dutifully downloaded and hid;
 * 3. **trouble outranks success**: a mascot showing "it worked" over a failed task hides
 *    exactly what people look at it for.
 */

import { screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { Settings, TaskDoneEvent, TaskProgressEvent } from "../../../shared/contract";

/**
 * Hoisted along with the mock itself: a `vi.mock` factory is evaluated before the file's
 * ordinary declarations, and a reference to a plain variable from inside it would not reach.
 *
 * **The handlers start as something that shouts, not as something that shrugs.** They used to
 * start as no-ops, and they were never reset between tests — so an event fired before the new
 * mascot had subscribed went to the previous test's unmounted one, changed nothing anybody
 * could see, and the run ended with "expected success, got idle" and nothing to say why. It
 * failed once in the Linux container and nowhere else, which is how a race behaves.
 *
 * So: a handler nobody has subscribed refuses the event out loud, `live` says whether anybody
 * is listening at all, and each unsubscribe removes only its own handler — the cleanup runs in
 * a microtask and can land after the next test has already subscribed.
 */
const shared = vi.hoisted(() => {
  // Returning `void` rather than the inferred `never`: a function that only ever throws
  // cannot otherwise stand in the same slot as a real handler.
  const stub =
    (what: string): ((e: unknown) => void) =>
    (_e: unknown) => {
      throw new Error(
        "a " +
          what +
          " event was fired while nothing was subscribed: the mascot had not " +
          "mounted yet, or had already gone",
      );
    };
  return {
    drawingAsked: vi.fn(),
    settingsGet: vi.fn(),
    stub,
    live: { progress: false, done: false },
    handlers: {
      progress: stub("progress"),
      done: stub("done"),
    },
  };
});

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      settingsGet: () => shared.settingsGet(),
      settingsSet: async (s: unknown) => s,
    },
    onTaskProgress: (h: (e: unknown) => void) => {
      shared.handlers.progress = h;
      shared.live.progress = true;
      return Promise.resolve(() => {
        if (shared.handlers.progress === h) {
          shared.handlers.progress = shared.stub("progress");
          shared.live.progress = false;
        }
      });
    },
    onTaskDone: (h: (e: unknown) => void) => {
      shared.handlers.done = h;
      shared.live.done = true;
      return Promise.resolve(() => {
        if (shared.handlers.done === h) {
          shared.handlers.done = shared.stub("done");
          shared.live.done = false;
        }
      });
    },
    onViewersUpdate: () => Promise.resolve(() => {}),
  };
});

vi.mock("../MascotDrawing", async () => {
  // What is counted is **the module being asked for at all**. The lazy load reaches here only
  // when the mascot really draws, so the counter is the answer to "was it loaded".
  shared.drawingAsked();
  return await vi.importActual<typeof import("../MascotDrawing")>("../MascotDrawing");
});

const { Mascot } = await import("../Mascot");
const { SettingsProvider } = await import("../../../app/settings");

const SETTINGS: Settings = {
  viewer_activity_threshold_s: 30,
  geo_refine_outside: false,
  concurrent_heavy_tasks: 1,
  mascot: true,
  animations: true,
  language: null,
  theme: null,
};

function progress(over: Partial<TaskProgressEvent> = {}): TaskProgressEvent {
  return {
    event: "progress",
    id: "t1",
    state: "running",
    progress: 0.5,
    stage: null,
    speed_bps: null,
    eta_s: null,
    ...over,
  };
}

function done(over: Partial<TaskDoneEvent> = {}): TaskDoneEvent {
  return { event: "done", id: "t1", state: "completed", error: null, notices: [], ...over };
}

function show() {
  return renderIn(
    <SettingsProvider>
      <Mascot />
    </SettingsProvider>,
  );
}

/**
 * Wait until this mascot is the one listening.
 *
 * Firing before it has subscribed is the whole of the flake this file used to have: the event
 * reached nobody, the mood stayed as it was, and the failure named a mood rather than a cause.
 */
async function subscribed() {
  await waitFor(() => {
    expect(shared.live.progress).toBe(true);
    expect(shared.live.done).toBe(true);
  });
}

describe("the mascot", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Nothing carried over from the test before: its mascot is gone, and its handler with it.
    shared.handlers.progress = shared.stub("progress");
    shared.handlers.done = shared.stub("done");
    shared.live.progress = false;
    shared.live.done = false;
    shared.settingsGet.mockResolvedValue(SETTINGS);
  });

  it("goes to work on a real task event and to worry on a failure", async () => {
    show();
    const drawing = await screen.findByTestId("mascot-drawing");
    expect(drawing).toHaveAttribute("data-mood", "idle");
    await subscribed();

    shared.handlers.progress(progress());
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "working"),
    );

    shared.handlers.done(done({ state: "failed", error: { code: "FFMPEG_BROKEN", details: [] } }));
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "trouble"),
    );
    // Out loud as well as in colour: to somebody listening to the screen a picture says
    // nothing at all.
    expect(screen.getByTestId("mascot-drawing")).toHaveAttribute(
      "aria-label",
      ru.ui.appearance.mascotTrouble,
    );
  });

  it("is pleased when a task really finishes", async () => {
    show();
    await screen.findByTestId("mascot-drawing");
    await subscribed();

    shared.handlers.progress(progress());
    shared.handlers.done(done());
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "success"),
    );
  });

  it("does not congratulate a cancelled task", async () => {
    // The person cancelled it themselves. Praising them for that would be absurd, and worrying
    // about it more so.
    show();
    await screen.findByTestId("mascot-drawing");
    await subscribed();

    shared.handlers.progress(progress());
    shared.handlers.done(done({ state: "cancelled" }));
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "idle"),
    );
  });

  it("shows nothing at all when it is turned off", async () => {
    // Only "not visible" here. "Not loaded" is in `mascot-off.test.tsx`, in a file of its own:
    // in this one the module registry is already warm from the tests above, and the request
    // counter would be answering about them rather than about this check. Verified: in a
    // shared file that check passes always.
    shared.settingsGet.mockResolvedValue({ ...SETTINGS, mascot: false });
    show();

    await waitFor(() => expect(shared.settingsGet).toHaveBeenCalled());
    expect(screen.queryByTestId("mascot-slot")).not.toBeInTheDocument();
  });
});
