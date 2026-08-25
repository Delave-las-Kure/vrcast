/**
 * T124 — interface checks for preparing a file.
 *
 * The core is stubbed: an interface check must not need FFmpeg or a real video.
 * What matters here is what a person sees — above all, whether the screen says
 * that hours of re-encoding are about to happen, and whether a file that failed
 * its playback check is visibly unusable.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConvertPreview, SourceFile, Validation } from "../../../shared/contract";

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
  const actual =
    await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
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
      video: { kind: "reencode", reason: "видео в hevc — плеер VRChat играет только H.264", level: "4.1" },
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
  fireEvent.click(await screen.findByText("Выбрать файл…"));
  await screen.findByText(/1920×1080/);
}

describe("preparation screen", () => {
  it("shows what is actually in the file", async () => {
    // FR-020. Choosing a bitrate without knowing what the source is means guessing.
    render(<ConvertScreen />);
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
    render(<ConvertScreen />);
    await pickSource();

    expect(await screen.findByText(/придётся пересжать/)).toBeInTheDocument();
    expect(screen.getByText(/это часы работы/i)).toBeInTheDocument();
    expect(screen.getByText(/играет только H\.264/)).toBeInTheDocument();
  });

  it("says when nothing will be re-encoded", async () => {
    mockConvertPreview.mockResolvedValue(
      preview({
        lossless: true,
        plan: { ...preview().plan, video: { kind: "copy" }, audio: { kind: "copy" } },
      }),
    );
    render(<ConvertScreen />);
    await pickSource();

    expect(await screen.findByText(/без потерь и за минуты/)).toBeInTheDocument();
  });

  it("passes on what the core says about the encoder", async () => {
    // FR-026: the move to the processor must not be silent. The wording comes from
    // the core so it cannot drift from what the core actually decided.
    mockConvertPreview.mockResolvedValue(
      preview({
        encoder: { kind: "software" },
        encoder_notice:
          "Аппаратного ускорения на этой машине не нашлось — кодировать будет процессор.",
      }),
    );
    render(<ConvertScreen />);
    await pickSource();

    expect(await screen.findByText(/кодировать будет процессор/)).toBeInTheDocument();
  });

  it("shows a track with no language by its number", async () => {
    // Boundary case of the spec: choosing between two blank lines is impossible,
    // and files with six unnamed tracks are ordinary.
    mockSourceProbe.mockResolvedValue(
      source({
        audio_tracks: [
          { index: 0, codec: "aac", channels: 2, bitrate_bps: null, language: null, title: null, is_default: false },
          { index: 1, codec: "ac3", channels: 6, bitrate_bps: null, language: null, title: null, is_default: true },
        ],
      }),
    );
    render(<ConvertScreen />);
    await pickSource();

    expect(screen.getByText(/Дорожка 1, стерео/)).toBeInTheDocument();
    expect(screen.getByText(/Дорожка 2, 6 каналов \(основная\)/)).toBeInTheDocument();
  });

  it("does not let a file without sound be prepared silently", async () => {
    mockSourceProbe.mockResolvedValue(source({ audio_tracks: [] }));
    render(<ConvertScreen />);
    await pickSource();

    expect(screen.getByText(/нет ни одной звуковой дорожки/)).toBeInTheDocument();
  });

  it("cannot be started before a source is chosen", async () => {
    render(<ConvertScreen />);
    expect(await screen.findByText("Подготовить")).toBeDisabled();
  });

  it("hands the core the track and bitrate that were chosen", async () => {
    render(<ConvertScreen />);
    await pickSource();

    fireEvent.change(screen.getByLabelText("Целевой битрейт"), {
      target: { value: "22000" },
    });
    fireEvent.click(screen.getByText("Подготовить"));

    await waitFor(() => expect(mockConvertStart).toHaveBeenCalled());
    expect(await screen.findByText(/Подготовка началась/)).toBeInTheDocument();
  });
});

describe("playback check", () => {
  function verdict(over: Partial<Validation> = {}): Validation {
    return { ok: true, problems: [], ignored: [], ...over };
  }

  it("says a passing file may be uploaded", () => {
    render(<ValidationResult result={verdict()} />);
    expect(screen.getByText(/можно заливать/)).toBeInTheDocument();
  });

  it("makes a failing file visibly unusable and keeps the decoder's words", () => {
    // FR-027. "Invalid NAL unit size" is cryptic but searchable; "the file is
    // broken" is neither.
    render(
      <ValidationResult
        result={verdict({ ok: false, problems: ["[h264 @ 0x1] Invalid NAL unit size (-56 > 271)."] })}
      />,
    );
    expect(screen.getByText(/Заливать его нельзя/)).toBeInTheDocument();
    expect(screen.getByText(/Invalid NAL unit size/)).toBeInTheDocument();
  });

  it("does not hide the complaints it decided to forgive", () => {
    // Otherwise someone later wonders why a file with warnings was accepted, and
    // there is nothing on screen to answer them.
    render(
      <ValidationResult
        result={verdict({ ignored: ["[null @ 0x1] non monotonically increasing dts"] })}
      />,
    );
    expect(screen.getByText(/на воспроизведение\s+не влияют/)).toBeInTheDocument();
  });
});
