/**
 * T696 (the owner's decision В3) — the «Субтитры» line of a video's plan: «без субтитров» by
 * default, a track by its language, title and «signs only» mark; the choice goes to the core,
 * and is not offered once encoding has begun, nor for a film with nothing that can be burned.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { en, renderIn, ru } from "../../../test-utils";
import type { SourceFile, SubtitleTrack, VideoView } from "../../../shared/contract";

const mockSet = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      videoSetSubtitles: (...a: unknown[]) => mockSet(...a),
    }),
  };
});

const { SubtitlePick } = await import("../SubtitlePick");

function track(index: number, over: Partial<SubtitleTrack> = {}): SubtitleTrack {
  return {
    index,
    codec: "subrip",
    kind: "text",
    language: "rus",
    title: null,
    forced: false,
    is_default: false,
    ...over,
  };
}

function source(subtitle_tracks: SubtitleTrack[]): SourceFile {
  return {
    path: "F:/films/Фильм.mkv",
    size_bytes: 20e9,
    duration_s: 7200,
    width: 1920,
    height: 1080,
    fps: 24,
    bitrate_bps: 8_000_000,
    peak_bps: null,
    video_codec: "h264",
    pix_fmt: "yuv420p",
    color_transfer: null,
    audio_tracks: [],
    subtitle_tracks,
  };
}

function video(over: Partial<VideoView> = {}): VideoView {
  return {
    id: "v1",
    server_id: "srv_1",
    source_path: "F:/films/Фильм.mkv",
    title: "Фильм",
    slug: "film",
    audio_track: 0,
    audio_chosen: true,
    subtitle_track: null,
    stage: "planned",
    state: "ready",
    paused_by_person: false,
    start_requested: false,
    source: source([
      track(0, { title: "Полные" }),
      track(1, { codec: "hdmv_pgs_subtitle", kind: "picture", language: "eng", forced: true }),
      track(2, { codec: "eia_608", kind: "other", language: null }),
    ]),
    plan: null,
    progress: null,
    task_id: null,
    media_id: null,
    problem: null,
    link: null,
    quality_links: [],
    created_at: "2026-10-01T10:00:00Z",
    updated_at: "2026-10-01T10:00:00Z",
    rev: 1,
    ...over,
  };
}

beforeEach(() => {
  mockSet.mockReset();
});

describe("the subtitles line of a plan (T696)", () => {
  it("offers «no subtitles» by default and every track that can be burned", () => {
    const run = vi.fn();
    renderIn(<SubtitlePick video={video()} busy={false} run={run} />);
    const select = screen.getByLabelText(ru.ui.video.subtitles) as HTMLSelectElement;
    expect(select.value).toBe("");
    const options = Array.from(select.options).map((o) => o.textContent);
    expect(options).toEqual([
      ru.ui.video.subtitlesNone,
      "rus — Полные",
      `eng${ru.ui.video.subtitlesForced}`,
    ]);
  });

  it("sends the choice to the core, and «no subtitles» as nothing", async () => {
    mockSet.mockResolvedValue(video({ subtitle_track: 1 }));
    const run = vi.fn((act: () => Promise<VideoView>) => void act());
    const { rerender } = renderIn(<SubtitlePick video={video()} busy={false} run={run} />);
    fireEvent.change(screen.getByLabelText(ru.ui.video.subtitles), { target: { value: "1" } });
    await waitFor(() => expect(mockSet).toHaveBeenCalledWith("v1", 1));

    rerender(<SubtitlePick video={video({ subtitle_track: 1 })} busy={false} run={run} />);
    expect((screen.getByLabelText(ru.ui.video.subtitles) as HTMLSelectElement).value).toBe("1");
    fireEvent.change(screen.getByLabelText(ru.ui.video.subtitles), { target: { value: "" } });
    await waitFor(() => expect(mockSet).toHaveBeenLastCalledWith("v1", null));
  });

  it("is not there for a film with nothing to burn, and cannot be changed once encoding began", () => {
    const { rerender } = renderIn(
      <SubtitlePick
        video={video({ source: source([track(0, { kind: "other" })]) })}
        busy={false}
        run={() => {}}
      />,
    );
    expect(screen.queryByLabelText(ru.ui.video.subtitles)).toBeNull();
    rerender(
      <SubtitlePick
        video={video({ stage: "encoding", state: "working" })}
        busy={false}
        run={() => {}}
      />,
    );
    expect(screen.getByLabelText(ru.ui.video.subtitles)).toBeDisabled();
  });

  it("speaks English too", () => {
    renderIn(<SubtitlePick video={video()} busy={false} run={() => {}} />, "en");
    expect(screen.getByLabelText(en.ui.video.subtitles)).toBeInTheDocument();
    expect(screen.getByText(en.ui.video.subtitlesNone)).toBeInTheDocument();
    expect(screen.getByText(`eng${en.ui.video.subtitlesForced}`)).toBeInTheDocument();
  });
});
