/**
 * T124 — interface checks for preparing a file.
 *
 * The core is stubbed: an interface check must not need FFmpeg or a real video.
 * What matters here is what a person sees — above all, whether the screen says
 * that hours of re-encoding are about to happen, and whether a file that failed
 * its playback check is visibly unusable.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ConvertPreview,
  ConvertStart,
  SourceFile,
  Validation,
} from "../../../shared/contract";
import { en, renderIn, ru } from "../../../test-utils";
import { fill } from "../../../shared/i18n/render";

const mockSourceProbe = vi.fn<() => Promise<SourceFile>>();
const mockConvertPreview = vi.fn<() => Promise<ConvertPreview>>();
const mockConvertStart = vi.fn<(request: ConvertStart) => Promise<string>>();

/** What the core would send when the preparation ends. Held so a test can end one. */
let finish: ((e: { id: string; state: string; error: unknown }) => void) | null = null;
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
      convertStart: (request: ConvertStart) => mockConvertStart(request),
      convertValidate: vi.fn(),
    },
    onTaskDone: async (handler: (e: unknown) => void) => {
      const mine = handler as typeof finish;
      finish = mine;
      // **Only its own.** A component unmounting from an earlier test runs its cleanup
      // whenever React gets round to it, and a plain `finish = null` there wipes out the
      // subscription the *current* test just made — so the event has nobody to reach and
      // the test fails on a timeout, sometimes. The same flake as the mascot's, 2026-08-28.
      return () => {
        if (finish === mine) finish = null;
      };
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>, "en");
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
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
    await pickSource();

    expect(screen.getByText(/Дорожка 1, стерео/)).toBeInTheDocument();
    expect(screen.getByText(/Дорожка 2, 6 каналов \(основная\)/)).toBeInTheDocument();
  });

  it("does not let a file without sound be prepared silently", async () => {
    mockSourceProbe.mockResolvedValue(source({ audio_tracks: [] }));
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
    await pickSource();

    expect(screen.getByText(ru.ui.convert.noTracks)).toBeInTheDocument();
  });

  it("cannot be started before a source is chosen", async () => {
    renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
    expect(await screen.findByText(ru.ui.convert.start)).toBeDisabled();
  });

  it("offers the next step, with the file it just made, once the work has ended", async () => {
    // Preparing used to be the end of the road: the path to the result lived in this
    // component and died with it, so the next screen asked for the file from scratch. And
    // there was nothing on screen to say the work had finished at all — "started" stayed up
    // for as long as the screen did.
    renderIn(
      <MemoryRouter>
        <ConvertScreen />
      </MemoryRouter>,
    );
    await pickSource();
    // **Wait for the button to be pressable.** The preview arrives a tick after the source
    // does, and the button is disabled until it has. Clicking earlier clicks nothing, the
    // task never starts, and the test fails somewhere further down on a timeout — which is
    // exactly how this one flaked, one run in five.
    await waitFor(() => expect(screen.getByText(ru.ui.convert.start)).toBeEnabled());
    fireEvent.click(screen.getByText(ru.ui.convert.start));
    await waitFor(() => expect(mockConvertStart).toHaveBeenCalled());

    // Nothing is offered while it is still running: a link to a file that is half written
    // is worse than no link.
    expect(screen.queryByTestId("what-next")).toBeNull();
    await waitFor(() => expect(finish).not.toBeNull());

    const made = mockConvertStart.mock.calls[0][0].out_path;
    finish?.({ id: await mockConvertStart.mock.results[0].value, state: "completed", error: null });

    const onward = await screen.findByText(ru.ui.convert.nextLadder);
    expect(onward.getAttribute("href")).toBe(`/ladder?file=${encodeURIComponent(made)}`);
    expect(screen.getByText(ru.ui.convert.nextUpload).getAttribute("href")).toBe(
      `/upload?file=${encodeURIComponent(made)}`,
    );
  });

  it("offers nothing onward when the preparation failed", async () => {
    // FR-027. A file that did not finish being written must not be offered to viewers, and
    // a link that says "send this" is an offer.
    renderIn(
      <MemoryRouter>
        <ConvertScreen />
      </MemoryRouter>,
    );
    await pickSource();
    // **Wait for the button to be pressable.** The preview arrives a tick after the source
    // does, and the button is disabled until it has. Clicking earlier clicks nothing, the
    // task never starts, and the test fails somewhere further down on a timeout — which is
    // exactly how this one flaked, one run in five.
    await waitFor(() => expect(screen.getByText(ru.ui.convert.start)).toBeEnabled());
    fireEvent.click(screen.getByText(ru.ui.convert.start));
    await waitFor(() => expect(finish).not.toBeNull());

    finish?.({
      id: await mockConvertStart.mock.results[0].value,
      state: "failed",
      error: { code: "CONVERT_FAILED", details: [] },
    });

    expect(await screen.findByTestId("what-next-failed")).toBeInTheDocument();
    expect(screen.queryByTestId("what-next")).toBeNull();
  });

  it("asks for no target bitrate — this screen makes a master, the ladder picks the rungs", async () => {
    // The field is gone (owner, 2026-08-28): asking for a number before anybody has looked
    // at the material is asking for a guess. What must not happen quietly is the screen
    // going on sending some number of its own.
    const { container } = renderIn(<MemoryRouter><ConvertScreen /></MemoryRouter>);
    await pickSource();

    // The screen really did draw its form — otherwise the next assertion would pass
    // because nothing was rendered at all.
    expect(screen.getByLabelText(ru.ui.convert.fieldTrack)).toBeInTheDocument();
    expect(container.querySelector("#convert-bitrate")).toBeNull();

    // **Wait for the button to be pressable.** The preview arrives a tick after the source
    // does, and the button is disabled until it has. Clicking earlier clicks nothing, the
    // task never starts, and the test fails somewhere further down on a timeout — which is
    // exactly how this one flaked, one run in five.
    await waitFor(() => expect(screen.getByText(ru.ui.convert.start)).toBeEnabled());
    fireEvent.click(screen.getByText(ru.ui.convert.start));
    await waitFor(() => expect(mockConvertStart).toHaveBeenCalled());
    expect(mockConvertStart.mock.calls[0][0]).toMatchObject({ target_kbps: null });
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
