/**
 * T693 (the owner's decision А1) — a set's leftover mp4 files: one line with the room they
 * take and «Remove», removed only on that press, and not offered while a video builds the set.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { en, renderIn, ru } from "../../../test-utils";
import type { FileView, MediaView } from "../../../shared/contract";

const mockRemove = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      mediaRemoveSetFiles: (...a: unknown[]) => mockRemove(...a),
    }),
  };
});

const { LeftoverRungFiles } = await import("../LeftoverRungFiles");

const GB = 1024 * 1024 * 1024;

function file(path: string, size_bytes: number, exists_on_server = true): FileView {
  return {
    path,
    size_bytes,
    duration_s: 3725,
    width: 1920,
    height: 1080,
    bitrate_bps: 9_000_000,
    video_codec: "h264",
    audio_codec: "aac",
    faststart_ok: true,
    exists_on_server,
    origin_url: `https://stream.example.com/videos/${path}`,
    cdn_url: null,
  };
}

function media(over: Partial<MediaView> = {}): MediaView {
  return {
    id: "m1",
    title: "Фильм",
    slug: "film",
    files: [],
    ladders: [],
    total_bytes: 0,
    created_at: "2026-10-01T10:00:00Z",
    set_files: [file("film_9.mp4", 4 * GB), file("film_4.mp4", 2 * GB)],
    ...over,
  };
}

beforeEach(() => {
  mockRemove.mockReset();
});

describe("leftover mp4 files of a set (T693)", () => {
  it("shows one line with the room they take, and removes them only on «Remove»", async () => {
    mockRemove.mockResolvedValue(6 * GB);
    const onRemoved = vi.fn();
    renderIn(<LeftoverRungFiles serverId="srv_1" media={media()} onRemoved={onRemoved} />);
    const line = screen.getByTestId("leftover-m1");
    expect(line.textContent).toMatch(/Лишние файлы mp4 — 6(,0)? ГБ/);
    expect(mockRemove).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(ru.ui.library.leftoverRemove));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
    expect(mockRemove).toHaveBeenCalledWith("srv_1", "m1");
  });

  it("counts only the files still on the server, and says it in English too", () => {
    renderIn(
      <LeftoverRungFiles
        serverId="srv_1"
        media={media({
          set_files: [file("film_9.mp4", 4 * GB), file("film_4.mp4", 2 * GB, false)],
        })}
        onRemoved={() => {}}
      />,
      "en",
    );
    expect(screen.getByTestId("leftover-m1").textContent).toMatch(/Extra mp4 files — 4(\.0)? GB/);
    expect(screen.getByText(en.ui.library.leftoverRemove)).toBeInTheDocument();
  });

  it("is not offered with no leftovers, nor while a video builds the set", () => {
    const { rerender } = renderIn(
      <LeftoverRungFiles serverId="srv_1" media={media({ set_files: [] })} onRemoved={() => {}} />,
    );
    expect(screen.queryByTestId("leftover-m1")).toBeNull();
    rerender(
      <LeftoverRungFiles
        serverId="srv_1"
        media={media({ set_work: { video_id: "v1", state: "building" } })}
        onRemoved={() => {}}
      />,
    );
    expect(screen.queryByTestId("leftover-m1")).toBeNull();
  });

  it("says a refusal in one line and keeps the files", async () => {
    mockRemove.mockRejectedValue({ code: "MEDIA_BUSY", details: [] });
    const onRemoved = vi.fn();
    renderIn(<LeftoverRungFiles serverId="srv_1" media={media()} onRemoved={onRemoved} />);
    fireEvent.click(screen.getByText(ru.ui.library.leftoverRemove));
    await screen.findByRole("alert");
    expect(onRemoved).not.toHaveBeenCalled();
  });
});
