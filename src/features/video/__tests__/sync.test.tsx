/**
 * T687 (QA-25 №8) — the «Video» screen does not lose a change between the list and the
 * subscription, does not roll fresh progress back with a late list, and catches up by itself
 * when an event was lost.
 *
 * The transport is controlled here: the subscription resolves when the test says so, and the
 * list answers what the test says — the order QA's probe used to show the race.
 */

import { act, cleanup, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn } from "../../../test-utils";
import type { ServerProfile, VideoView } from "../../../shared/contract";
import { mergeListed, upsert } from "../rules";

const wire = vi.hoisted(() => ({
  list: vi.fn<() => Promise<VideoView[]>>(),
  servers: vi.fn<() => Promise<ServerProfile[]>>(),
  /** Resolves the screen's subscription to `video:update`. */
  connect: null as null | (() => void),
  push: null as null | ((v: VideoView) => void),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      videoList: () => wire.list(),
      serversList: () => wire.servers(),
    }),
    onVideoUpdate: (handler: (v: VideoView) => void) =>
      new Promise<() => void>((resolve) => {
        wire.connect = () => {
          wire.push = handler;
          resolve(() => {
            if (wire.push === handler) wire.push = null;
          });
        };
      }),
    onVideoRemoved: async () => () => {},
  };
});

const { VideoScreen } = await import("../VideoScreen");
const { useServers } = await import("../../servers/store");

function video(over: Partial<VideoView> = {}): VideoView {
  return {
    id: "v",
    server_id: "srv",
    title: "Фильм",
    slug: "film",
    source_path: "film.mp4",
    audio_track: 0,
    state: "working",
    stage: "encoding",
    paused_by_person: false,
    start_requested: true,
    source: null,
    plan: null,
    task_id: "t",
    media_id: "m",
    problem: null,
    link: null,
    created_at: "2026-10-02T01:00:00.000000000Z",
    updated_at: "2026-10-02T01:00:00.000000001Z",
    rev: 10,
    progress: {
      task_state: "running",
      progress: 0.1,
      speed_bps: null,
      eta_s: null,
      rung: 1,
      rungs: 4,
    },
    ...over,
  };
}

function show() {
  return renderIn(
    <MemoryRouter>
      <VideoScreen />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  wire.list.mockReset();
  wire.servers.mockReset();
  wire.push = null;
  wire.connect = null;
  const p = { id: "srv", name: "QA", is_active: true } as ServerProfile;
  useServers.setState({ profiles: [p], loading: false, error: null });
  wire.servers.mockResolvedValue([p]);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("merging by version (T687)", () => {
  it("a late list does not roll fresh progress back, at the same updated_at", () => {
    const old = video({ rev: 10 });
    const fresh = video({ rev: 11, progress: { ...old.progress!, progress: 0.7 } });
    expect(mergeListed([fresh], [old])[0].progress?.progress).toBe(0.7);
    expect(upsert([fresh], old)[0].progress?.progress).toBe(0.7);
    expect(upsert([old], fresh)[0].progress?.progress).toBe(0.7);
  });

  it("a newer view wins whatever its updated_at says", () => {
    const listed = video({ rev: 20, state: "done", stage: "done", updated_at: "2026-10-02T01" });
    const event = video({ rev: 19, updated_at: "2026-10-02T02" });
    expect(upsert([listed], event)[0].state).toBe("done");
  });
});

describe("the screen's snapshot and its events (T687)", () => {
  it("asks for the list only once it is listening, so a change in between is not lost", async () => {
    wire.list.mockResolvedValue([video()]);
    show();
    // Not before the subscription is there: what ends in between would never be heard.
    await act(async () => {
      await Promise.resolve();
    });
    expect(wire.list).not.toHaveBeenCalled();
    // The work ends now; the list taken after the subscription says so.
    wire.list.mockResolvedValue([video({ state: "done", stage: "done", progress: null, rev: 12 })]);
    await act(async () => wire.connect!());
    await waitFor(() => expect(screen.getByTestId("video-v")).toHaveClass("video--done"));
  });

  it("asks again when the window comes back, and catches a change it never heard of", async () => {
    wire.list.mockResolvedValue([video()]);
    show();
    await act(async () => wire.connect!());
    await waitFor(() => expect(screen.getByTestId("video-v")).toHaveClass("video--working"));
    wire.list.mockResolvedValue([video({ state: "done", stage: "done", progress: null, rev: 30 })]);
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(screen.getByTestId("video-v")).toHaveClass("video--done"));
  });

  it("asks again by itself every little while as long as something is in work", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    wire.list.mockResolvedValue([video()]);
    show();
    await act(async () => wire.connect!());
    await vi.waitFor(() => expect(wire.list).toHaveBeenCalledTimes(1));
    wire.list.mockResolvedValue([video({ state: "done", stage: "done", progress: null, rev: 40 })]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(16_000);
    });
    await vi.waitFor(() => expect(screen.getByTestId("video-v")).toHaveClass("video--done"));
    const asked = wire.list.mock.calls.length;
    // Nothing in work any more: it stops asking.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(wire.list.mock.calls.length).toBe(asked);
  });

  it("does not let a late list undo an event that came after it was asked for", async () => {
    let answer: (v: VideoView[]) => void = () => {};
    wire.list.mockReturnValue(new Promise((r) => (answer = r)));
    show();
    await act(async () => wire.connect!());
    // The event (newer) arrives first; the list (taken before it) arrives late.
    act(() => wire.push!(video({ rev: 50, progress: { ...video().progress!, progress: 0.7 } })));
    await act(async () => answer([video({ rev: 49 })]));
    expect(screen.getByTestId("stage-facts")).toHaveTextContent("70%");
  });
});
