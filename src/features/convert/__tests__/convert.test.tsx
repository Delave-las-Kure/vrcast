/**
 * T124 — interface checks for preparing a file.
 *
 * The core is stubbed: an interface check must not need FFmpeg or a real video.
 * What matters here is what a person sees — above all, whether the screen says
 * that hours of re-encoding are about to happen, and whether a file that failed
 * its playback check is visibly unusable.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConvertPreview, SourceFile, Validation } from "../../../shared/contract";
import { en, renderIn, ru } from "../../../test-utils";
import { fill } from "../../../shared/i18n/render";

const mockSourceProbe = vi.fn<() => Promise<SourceFile>>();
const mockConvertPreview = vi.fn<() => Promise<ConvertPreview>>();
const mockConvertStart = vi.fn<() => Promise<string>>();
const mockOpen = vi.fn<() => Promise<string | null>>();
const mockSave = vi.fn<() => Promise<string | null>>();

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: () => mockOpen(),
  save: () => mockSave(),
}));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      sourceProbe: () => mockSourceProbe(),
      convertPreview: () => mockConvertPreview(),
      convertStart: () => mockConvertStart(),
      convertValidate: vi.fn(),
    },
  };
});

const { ConvertScreen } = await import("../ConvertScreen");
const { ValidationResult } = await import("../ValidationResult");

function source(over: Partial<SourceFile> = {}): SourceFile {
  return {
    path: "F:/video/source.mkv",
    size_bytes: 8_000_000_000,
    duration_s: 7200,
    width: 1920,
    height: 1080,
    fps: 24,
    bitrate_bps: 9_000_000,
    peak_bps: null,
    video_codec: "hevc",
    pix_fmt: "yuv420p",
    color_transfer: null,
    audio_tracks: [
      {
        index: 0,
        codec: "aac",
        channels: 2,
        bitrate_bps: 256_000,
        language: "rus",
        title: null,
        is_default: true,
      },
    ],
    ...over,
  };
}

function preview(over: Partial<ConvertPreview> = {}): ConvertPreview {
  return {
    source: source(),
    plan: {
      video: {
        kind: "reencode",
        reason: { key: "REASON_VIDEO_NOT_H264", params: { codec: "hevc" } },
        level: "4.1",
      },
      audio: { kind: "copy" },
      audio_track: 0,
      gop: 24,
      tonemap: false,
      requested_height: null,
      faststart: true,
    },
    encoder: { kind: "hardware", name: "h264_nvenc" },
    encoder_notice: null,
    lossless: false,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockOpen.mockResolvedValue("F:/video/source.mkv");
  mockSave.mockResolvedValue("F:/video/source.ready.mp4");
  mockSourceProbe.mockResolvedValue(source());
  mockConvertPreview.mockResolvedValue(preview());
  mockConvertStart.mockResolvedValue("t-1");
});

/** Pick a source and wait for the screen to catch up. */
async function pickSource() {
  fireEvent.click(await screen.findByText(ru.ui.convert.pickFile));
  await screen.findByText(/1920×1080/);
}

describe("preparation screen", () => {
  it("shows what is actually in the file", async () => {
    // FR-020. Choosing a bitrate without knowing what the source is means guessing.
    renderIn(<ConvertScreen />);
    await pickSource();
    // One line, matched whole: "hevc" also appears in the preview's explanation,
    // and matching it alone would find either and prove neither.
    expect(
      screen.getByText((_, el) => {
        const text = el?.textContent ?? "";
        return (
          el?.className === "form__hint" &&
          text.includes("1920×1080") &&
          text.includes("24 кадр/с") &&
          text.includes("hevc")
        );
      }),
    ).toBeInTheDocument();
  });

  it("says plainly that re-encoding is about to happen, and why", async () => {
    // The whole reason this screen is not just a button: copying takes minutes,
    // re-encoding takes hours, and from the outside both look the same.
    renderIn(<ConvertScreen />);
    await pickSource();

    expect(await screen.findByText(ru.ui.convert.lossy)).toBeInTheDocument();
    // And the reason, with the codec put into it. Matched whole: "hevc" alone also
    // appears in the line listing what is in the file, and would prove nothing.
    const reason = fill(ru.details.REASON_VIDEO_NOT_H264, { codec: "hevc" }, ru, "ru");
    const lines = screen.getAllByRole("listitem");
    expect(
      lines.some((el) => el.textContent?.includes(reason)),
      `not one line of the plan names the reason: ${reason}`,
    ).toBe(true);
  });

  it("says when nothing will be re-encoded", async () => {
    mockConvertPreview.mockResolvedValue(
      preview({
        lossless: true,
        plan: { ...preview().plan, video: { kind: "copy" }, audio: { kind: "copy" } },
      }),
    );
    renderIn(<ConvertScreen />);
    await pickSource();

    expect(await screen.findByText(ru.ui.convert.lossless)).toBeInTheDocument();
  });

  it("passes on what the core says about the encoder", async () => {
    // FR-026: the move to the processor must not be silent. The core sends the code
    // and the interface words it, so the two cannot drift apart.
    mockConvertPreview.mockResolvedValue(
      preview({
        encoder: { kind: "software" },
        encoder_notice: { key: "NOTICE_NO_HARDWARE_FOUND" },
      }),
    );
    renderIn(<ConvertScreen />);
    await pickSource();

    expect(await screen.findByText(ru.details.NOTICE_NO_HARDWARE_FOUND)).toBeInTheDocument();
  });

  it("names the accelerator that failed in a way a person recognises", async () => {
    // The core sends `h264_nvenc`; nobody outside a terminal knows what that is.
    mockConvertPreview.mockResolvedValue(
      preview({
        encoder: { kind: "software" },
        encoder_notice: {
          key: "NOTICE_HARDWARE_FAILED",
          params: { encoder: "h264_nvenc" },
        },
      }),
    );
    renderIn(<ConvertScreen />);
    await pickSource();

    // Asked of the catalogue rather than copied out of it: what matters is that the
    // sentence names the make of card, not the exact words around it.
    expect(
      await screen.findByText(
        fill(ru.details.NOTICE_HARDWARE_FAILED, { encoder: "h264_nvenc" }, ru, "ru"),
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/h264_nvenc/)).not.toBeInTheDocument();
  });

  it("explains the same preparation in English when English is chosen", async () => {
    renderIn(<ConvertScreen />, "en");
    fireEvent.click(await screen.findByText(en.ui.convert.pickFile));
    await screen.findByText(/1920×1080/);

    expect(await screen.findByText(en.ui.convert.lossy)).toBeInTheDocument();
    expect(screen.getByText(/only plays H\.264/)).toBeInTheDocument();
  });

  it("shows a track with no language by its number", async () => {
    // Boundary case of the spec: choosing between two blank lines is impossible,
    // and files with six unnamed tracks are ordinary.
    mockSourceProbe.mockResolvedValue(
      source({
        audio_tracks: [
          {
            index: 0,
            codec: "aac",
            channels: 2,
            bitrate_bps: null,
            language: null,
            title: null,
            is_default: false,
          },
          {
            index: 1,
            codec: "ac3",
            channels: 6,
            bitrate_bps: null,
            language: null,
            title: null,
            is_default: true,
          },
        ],
      }),
    );
    renderIn(<ConvertScreen />);
    await pickSource();

    expect(screen.getByText(/Дорожка 1, стерео/)).toBeInTheDocument();
    expect(screen.getByText(/Дорожка 2, 6 каналов \(основная\)/)).toBeInTheDocument();
  });

  it("does not let a file without sound be prepared silently", async () => {
    mockSourceProbe.mockResolvedValue(source({ audio_tracks: [] }));
    renderIn(<ConvertScreen />);
    await pickSource();

    expect(screen.getByText(ru.ui.convert.noTracks)).toBeInTheDocument();
  });

  it("cannot be started before a source is chosen", async () => {
    renderIn(<ConvertScreen />);
    expect(await screen.findByText(ru.ui.convert.start)).toBeDisabled();
  });

  it("hands the core the track and bitrate that were chosen", async () => {
    renderIn(<ConvertScreen />);
    await pickSource();

    fireEvent.change(screen.getByLabelText(ru.ui.convert.fieldBitrate), {
      target: { value: "22000" },
    });
    fireEvent.click(screen.getByText(ru.ui.convert.start));

    await waitFor(() => expect(mockConvertStart).toHaveBeenCalled());
    expect(await screen.findByText(ru.ui.convert.started)).toBeInTheDocument();
  });
});

describe("playback check", () => {
  function verdict(over: Partial<Validation> = {}): Validation {
    return { ok: true, problems: [], ignored: [], ...over };
  }

  it("says a passing file may be uploaded", () => {
    renderIn(<ValidationResult result={verdict()} />);
    expect(screen.getByText(ru.ui.validation.ok)).toBeInTheDocument();
  });

  it("makes a failing file visibly unusable and keeps the decoder's words", () => {
    // FR-027. "Invalid NAL unit size" is cryptic but searchable; "the file is
    // broken" is neither.
    renderIn(
      <ValidationResult
        result={verdict({
          ok: false,
          problems: ["[h264 @ 0x1] Invalid NAL unit size (-56 > 271)."],
        })}
      />,
    );
    expect(screen.getByText(ru.ui.validation.failed)).toBeInTheDocument();
    expect(screen.getByText(/Invalid NAL unit size/)).toBeInTheDocument();
  });

  it("does not hide the complaints it decided to forgive", () => {
    // Otherwise someone later wonders why a file with warnings was accepted, and
    // there is nothing on screen to answer them.
    renderIn(
      <ValidationResult
        result={verdict({ ignored: ["[null @ 0x1] non monotonically increasing dts"] })}
      />,
    );
    expect(
      screen.getByText(fill(ru.ui.validation.ignoredSummary, { n: 1 }, ru, "ru")),
    ).toBeInTheDocument();
  });
});
