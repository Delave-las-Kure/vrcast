/**
 * T202, T242 — the quality-set screen.
 *
 * What is checked is what this screen exists to prevent: a guess shown as a measurement, a
 * build offered on rungs nobody has looked at, an objection saved up until after the person
 * has committed hours, and a promise about how long a measurement takes that hides whose
 * machine it was measured on.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type {
  LadderPreview,
  LadderVerdict,
  MeasurePreview,
  Rung,
  SourceMeasured,
} from "../../../shared/contract";

const mockLadderPlan = vi.fn<() => Promise<LadderPreview>>();
const mockLadderMeasure = vi.fn<() => Promise<SourceMeasured>>();
const mockLadderValidate = vi.fn<() => Promise<LadderVerdict>>();
const mockMeasurePreview = vi.fn<() => Promise<MeasurePreview>>();
const mockMeasureStart = vi.fn<() => Promise<string>>();
const mockBuild = vi.fn<(...a: unknown[]) => Promise<string>>();

/** What the core would send when a task ends. Held so a test can end one when it likes. */
let finish: ((e: { id: string; state: string; error: unknown }) => void) | null = null;

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      ladderPlan: () => mockLadderPlan(),
      ladderMeasure: () => mockLadderMeasure(),
      ladderValidate: () => mockLadderValidate(),
      qualityMeasurePreview: () => mockMeasurePreview(),
      qualityMeasureStart: () => mockMeasureStart(),
      ladderBuild: (...a: unknown[]) => mockBuild(...a),
    },
    onTaskDone: async (handler: (e: unknown) => void) => {
      finish = handler as typeof finish;
      return () => {
        finish = null;
      };
    },
  };
});

const { LadderScreen } = await import("../LadderScreen");

function rung(index: number, mbps: number, height: number, vmaf: number | null): Rung {
  return {
    index,
    bitrate_bps: mbps * 1_000_000,
    maxrate_bps: mbps * 1_100_000,
    bufsize_bps: mbps * 1_100_000,
    width: Math.round((height * 16) / 9),
    height,
    level: "5.1",
    reasons: vmaf === null ? ["step_down"] : ["measured_optimum"],
    quality:
      vmaf === null
        ? { state: "not_measured" }
        : { state: "measured_here", vmaf_x100: Math.round(vmaf * 100) },
  };
}

const SOURCE = {
  width: 3840,
  height: 2160,
  fps: 24,
  bitrate_bps: 60_000_000,
  heavier_codec: false,
  native_height: null,
};

function preview(
  from: LadderPreview["from"],
  rungs: Rung[],
  notBuildable: LadderPreview["verdict"]["not_buildable"] = null,
): LadderPreview {
  return {
    plan: { rungs, shape: "flat", anchor_bps: rungs[0]?.bitrate_bps ?? 0 },
    from,
    source: SOURCE,
    anchor_mmbps: null,
    anchor_mbps: from === "formula" ? 22 : null,
    verdict: { objections: [], not_buildable: notBuildable },
    notices: [],
  } as unknown as LadderPreview;
}

const MEASURED = [rung(0, 22, 2160, 96.1), rung(1, 12, 1440, 92.0), rung(2, 6, 1080, 87.4)];
const GUESSED = [rung(0, 22, 2160, null), rung(1, 12, 1440, null)];

beforeEach(() => {
  vi.clearAllMocks();
  mockLadderMeasure.mockResolvedValue({
    average_bps: 8_000_000,
    peak_bps: 41_000_000,
    worst: [],
    seconds: 3600,
  });
  mockLadderValidate.mockResolvedValue({ objections: [], not_buildable: null });
  mockMeasureStart.mockResolvedValue("task-1");
  mockBuild.mockResolvedValue("build-1");
  finish = null;
});

describe("where the rungs came from", () => {
  it("says plainly when they were measured on this material", async () => {
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "ru");

    await waitFor(() =>
      expect(screen.getByTestId("provenance")).toHaveTextContent(ru.ui.ladder.fromMeasured),
    );
  });

  it("says a formula ladder is a guess, and does not let it be built", async () => {
    // The whole reason the measurement exists (R-21). A guess shown the way a measurement
    // is shown is worse than no number at all.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "ru");

    await waitFor(() =>
      expect(screen.getByTestId("provenance")).toHaveTextContent(ru.ui.ladder.fromFormula),
    );
    expect(screen.getByTestId("build")).toBeDisabled();
    expect(screen.getByTestId("build-blocked")).toHaveTextContent(ru.ui.ladder.buildBlocked);
  });

  it("says a borrowed measurement is borrowed", async () => {
    // It is enough to build on — the next episode of a season is the same source — but it
    // is not a measurement of THIS file, and a person is owed the difference.
    mockLadderPlan.mockResolvedValue(preview("borrowed", MEASURED));
    renderIn(<LadderScreen path="F:/films/s01e02.mp4" />, "en");

    await waitFor(() =>
      expect(screen.getByTestId("provenance")).toHaveTextContent(en.ui.ladder.fromBorrowed),
    );
  });
});

describe("what a rung is worth", () => {
  it("shows the measured quality of each rung and marks the ones without", async () => {
    mockLadderPlan.mockResolvedValue(
      preview("measured", [rung(0, 22, 2160, 96.1), rung(1, 12, 1440, null)]),
    );
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("rung-0")).toBeInTheDocument());
    expect(screen.getByTestId("rung-0")).toHaveTextContent("96.10");
    expect(screen.getByTestId("rung-1")).toHaveTextContent(en.ui.ladder.notMeasured);
  });

  it("marks a rung edited by hand as no longer measured", async () => {
    // Moving a rung takes it off the grid that was measured. Nobody has looked at what it
    // is worth at its new value, and saying otherwise is the one lie here a person would
    // believe.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("rung-1")).toHaveTextContent("92.00"));
    fireEvent.change(screen.getByLabelText(`${en.ui.ladder.columnBitrate} 2`), {
      target: { value: "14" },
    });

    await waitFor(() =>
      expect(screen.getByTestId("rung-1")).toHaveTextContent(en.ui.ladder.notMeasured),
    );
  });

  it("shows an objection as soon as an edit makes one, without waiting for a build", async () => {
    // FR-044. Learning that a rung is impossible after agreeing to hours of encoding is
    // learning it too late.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");
    await waitFor(() => expect(screen.getByTestId("rung-0")).toBeInTheDocument());

    mockLadderValidate.mockResolvedValue({
      objections: [{ RungAboveSource: { index: 0, source_bps: 60_000_000 } }],
      not_buildable: null,
    });
    fireEvent.change(screen.getByLabelText(`${en.ui.ladder.columnBitrate} 1`), {
      target: { value: "90" },
    });

    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("above the source"));
  });
});

describe("what a measurement will cost", () => {
  it("says how long before it is started, and whose machine that is from", async () => {
    // FR-147, and the second half matters as much as the first: the difference between
    // twenty minutes and two hours is the whole decision.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("how-long")).toHaveTextContent("3 min"));
    expect(screen.getByTestId("how-long")).toHaveTextContent("12 points");
    expect(screen.getByTestId("estimate-from")).toHaveTextContent(en.ui.ladder.estimateFromModel);
  });

  it("counts only what is left when some of the grid is already measured", async () => {
    // A cancelled measurement costs the points not yet taken and nothing more, and the
    // offer to carry on has to say so rather than asking for the whole half hour again.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 9,
      about_seconds: 60,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 40,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("how-long")).toHaveTextContent("3 points"));
    expect(screen.getByTestId("how-long")).toHaveTextContent("of 12");
    expect(screen.getByTestId("estimate-from")).toHaveTextContent("40 points");
  });

  it("starts the measurement and says the work is not lost by leaving", async () => {
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));

    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("status")).toHaveTextContent(en.ui.ladder.measureRunning);
  });
});

describe("the source itself", () => {
  it("shows the peak rather than only the average", async () => {
    // FR-040. A connection has to hold the peak: a film that averages 8 and reaches 41 in
    // one scene freezes everyone under 41 when that scene arrives.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() =>
      expect(screen.getByTestId("source-facts")).toHaveTextContent("41.0 Mbit/s"),
    );
  });
});

describe("when the measurement ends", () => {
  it("shows the rungs it chose without waiting to be reopened", async () => {
    // The whole reason the screen listens at all. Without it the task runs to its end,
    // the rungs sit in the store, and this goes on saying "measuring" until somebody
    // thinks to close it and open it again.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" serverId="s1" />, "en");

    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));
    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalled());
    // And wait until the screen is actually listening. Firing before it is subscribed
    // makes the check pass for the wrong reason: nothing happens, and "nothing happened"
    // is exactly what two of these tests are looking for.
    await waitFor(() => expect(finish).not.toBeNull());

    // The task ends, and the core now has a measured ladder to give.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    finish?.({ id: "task-1", state: "completed", error: null });

    await waitFor(() =>
      expect(screen.getByTestId("provenance")).toHaveTextContent(en.ui.ladder.fromMeasured),
    );
    expect(screen.getByTestId("build")).toBeEnabled();
  });

  it("fills the rungs in when the core answers a tick later, not in the same microtask", async () => {
    // **The test above passes on broken code, and this is why it exists.**
    //
    // `mockResolvedValue` hands back an already-settled promise, so its `.then` runs in a
    // microtask — before React has flushed a single passive effect. The real
    // `ladder_plan` runs ffprobe and reads the database: tens to hundreds of milliseconds.
    //
    // In that gap the screen throws the answer away. The `task:done` handler sets
    // `measuring` to null first, which is the listening effect's own dependency, so React
    // tears the effect down and its cleanup sets `alive = false`. When the core finally
    // answers, `if (!alive) return;` drops the measured rungs on the floor — and the
    // `.catch` is behind the same flag, so a failure is silent too. The owner saw exactly
    // this on 2026-08-28: the measurement finished and was written down, and the screen
    // went on showing the guess with the build button disabled.
    //
    // The only difference from the test above is that the core is made to answer like the
    // core.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" serverId="s1" />, "en");

    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));
    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalled());
    await waitFor(() => expect(finish).not.toBeNull());

    mockLadderPlan.mockImplementation(
      () =>
        new Promise((resolve) => {
          setTimeout(() => resolve(preview("measured", MEASURED)), 10);
        }),
    );
    finish?.({ id: "task-1", state: "completed", error: null });

    await waitFor(() =>
      expect(screen.getByTestId("provenance")).toHaveTextContent(en.ui.ladder.fromMeasured),
    );
    expect(screen.getByTestId("build")).toBeEnabled();
  });

  it("says so when the reload after a measurement fails, instead of going quiet", async () => {
    // The other half of the same flaw: the `.catch` was behind the same flag as the answer,
    // so a measurement that finished and a reload that failed left the screen showing the
    // guess with nothing to say why.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));
    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalled());
    await waitFor(() => expect(finish).not.toBeNull());

    mockLadderPlan.mockImplementation(
      () =>
        new Promise((_resolve, reject) => {
          setTimeout(() => reject({ code: "LADDER_NOT_BUILDABLE", details: [] }), 10);
        }),
    );
    finish?.({ id: "task-1", state: "completed", error: null });

    await waitFor(() => expect(screen.getByRole("alert")).not.toBeNull());
  });

  it("says why the set cannot be built when no server is chosen", async () => {
    // The button used to be offered, pressed, and to do nothing at all: the handler
    // returned on the spot when there was no server. Indistinguishable, from the person's
    // side, from a build that started and failed silently.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("build-no-server")).not.toBeNull());
    expect(screen.getByTestId("build")).toBeDisabled();
    fireEvent.click(screen.getByTestId("build"));
    expect(mockBuild).not.toHaveBeenCalled();
  });

  it("does not reload when somebody else's task ends", async () => {
    // A person may have a preparation and a transfer running beside this. Reloading on
    // any task at all would be a flicker at best and a set appearing out of nowhere at
    // worst.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");
    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));
    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalled());
    // And wait until the screen is actually listening. Firing before it is subscribed
    // makes the check pass for the wrong reason: nothing happens, and "nothing happened"
    // is exactly what two of these tests are looking for.
    await waitFor(() => expect(finish).not.toBeNull());

    const asked = mockLadderPlan.mock.calls.length;
    finish?.({ id: "somebody-elses-task", state: "completed", error: null });
    await new Promise((r) => setTimeout(r, 20));
    expect(mockLadderPlan.mock.calls.length).toBe(asked);
  });

  it("says so when the measurement failed rather than going quiet", async () => {
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 180,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: "h264_nvenc",
      estimate_from_points: 0,
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");
    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));
    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalled());
    // And wait until the screen is actually listening. Firing before it is subscribed
    // makes the check pass for the wrong reason: nothing happens, and "nothing happened"
    // is exactly what two of these tests are looking for.
    await waitFor(() => expect(finish).not.toBeNull());

    finish?.({
      id: "task-1",
      state: "failed",
      error: { code: "VMAF_UNAVAILABLE", details: [] },
    });
    await waitFor(() =>
      expect(screen.getByText(en.errors.VMAF_UNAVAILABLE.message)).toBeInTheDocument(),
    );
  });
});

describe("choosing which rungs to build", () => {
  it("builds only the rungs left ticked", async () => {
    // The application works the ladder out and offers it; which of the rungs are actually
    // made is the person's to say (owner, 2026-08-28). Until now the only choice was to
    // retype the numbers by hand, and every rung is hours of encoding and a copy on the
    // server.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" serverId="s1" slug="film" />, "en");

    await waitFor(() => expect(screen.getByTestId("build")).toBeEnabled());
    fireEvent.click(screen.getByLabelText(en.ui.ladder.buildThisRung.replace("{mbps}", "12")));
    expect(screen.getByTestId("rung-1")).toHaveAttribute("data-left-out", "yes");

    fireEvent.click(screen.getByTestId("build"));
    await waitFor(() => expect(mockBuild).toHaveBeenCalledTimes(1));

    const sent = (mockBuild.mock.calls[0][0] as { rungs: Rung[] }).rungs;
    expect(sent.map((r) => r.bitrate_bps / 1_000_000)).toEqual([22, 6]);
  });

  it("cannot be built when every rung has been left out", async () => {
    // Nothing to build is not the same as ready to build, and a button that starts a task
    // producing no variants would be the worst kind of success.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" serverId="s1" slug="film" />, "en");

    await waitFor(() => expect(screen.getByTestId("build")).toBeEnabled());
    for (const mbps of ["22", "12", "6"]) {
      fireEvent.click(screen.getByLabelText(en.ui.ladder.buildThisRung.replace("{mbps}", mbps)));
    }

    expect(screen.getByTestId("build")).toBeDisabled();
  });
});

describe("what the set is called", () => {
  it("is offered rather than decided, and what is typed is what is built", async () => {
    // The guess comes from the file's own name, and a name with anything but Latin in it
    // guesses down to something nobody meant — which is not obvious until the set is
    // somewhere nobody expected.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" serverId="s1" slug="film" />, "en");

    await waitFor(() => expect(screen.getByTestId("build")).toBeEnabled());
    fireEvent.change(screen.getByLabelText(en.ui.ladder.setName), {
      target: { value: "blue-eye-s01e01" },
    });
    fireEvent.click(screen.getByTestId("build"));

    await waitFor(() => expect(mockBuild).toHaveBeenCalledTimes(1));
    expect(mockBuild.mock.calls[0][0]).toMatchObject({
      // snake_case, because that is what the core reads. It said `serverId` here until
      // 2026-08-28, agreed with the screen, and the two of them agreed on something the
      // core refused outright: `missing field server_id`, on every press of the button.
      server_id: "s1",
      slug: "blue-eye-s01e01",
    });
  });
});
