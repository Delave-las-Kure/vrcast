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
  Detail,
  LadderPreview,
  LadderVerdict,
  MeasurementView,
  MeasurePreview,
  Rung,
  SourceMeasured,
} from "../../../shared/contract";

const mockMeasureResult = vi.fn<() => Promise<MeasurementView>>();
const mockLadderPlan = vi.fn<() => Promise<LadderPreview>>();
const mockLadderMeasure = vi.fn<() => Promise<SourceMeasured>>();
const mockLadderValidate = vi.fn<() => Promise<LadderVerdict>>();
const mockMeasurePreview = vi.fn<() => Promise<MeasurePreview>>();
const mockMeasureStart = vi.fn<() => Promise<string>>();
const mockBuild = vi.fn<(...a: unknown[]) => Promise<string>>();

/** What the core would send when a task ends. Held so a test can end one when it likes.
 *
 * `notices` is optional here and required in the contract: most of these tests are about
 * something else and say nothing about it, and the one that is about it says it. */
let finish:
  ((e: { id: string; state: string; error: unknown; notices?: Detail[] }) => void) | null = null;

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Imported here rather than at the top: `vi.mock` is hoisted above every import in the
  // file, so a name from one is not initialised yet when this runs.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    // **Built from the real `ipc` rather than listed by hand** (T470). This screen grew a
    // call to `qualityMeasurements` when `Borrow` arrived, and a hand-written object that
    // did not have it threw inside an effect and took twenty-four tests down with it. What
    // is named below is what this file is about; everything else answers and gets out of
    // the way.
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      ladderPlan: () => mockLadderPlan(),
      qualityMeasureResult: () => mockMeasureResult(),
      ladderMeasure: () => mockLadderMeasure(),
      ladderValidate: () => mockLadderValidate(),
      qualityMeasurePreview: () => mockMeasurePreview(),
      qualityMeasureStart: () => mockMeasureStart(),
      ladderBuild: (...a: unknown[]) => mockBuild(...a),
    }),
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
  // **Every stub gets an answer here, not only the ones a given test reads.**
  // `clearAllMocks` takes the implementation away, so a stub left without one returns
  // `undefined`, and whatever calls it does `.then` on nothing. Which caller, and when,
  // depends on when React gets round to an effect — sometimes one belonging to a test that
  // has already ended — so it passes on one machine and fails on another. It did: green
  // here, red in CI on 2026-08-28. Two stubs were short; the answer is that none is.
  mockMeasurePreview.mockResolvedValue({
    source_key: "1:film.mp4",
    points: 12,
    already_measured: 0,
    about_seconds: 600,
    chunk_starts: [233, 590, 947],
    anchor_mbps: 8,
    encoder: { kind: "hardware", name: "h264_nvenc" },
    machine: { state: "nothing_timed_yet" },
    notices: [],
  } as unknown as MeasurePreview);
  mockMeasureResult.mockResolvedValue({
    run: { source_key: "1:film.mp4", codec: "h264" },
    points: [],
    selection: null,
    ladder: null,
    notices: [],
  } as unknown as MeasurementView);
  mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
  finish = null;
});

describe("what was actually measured", () => {
  // T420, T421. The core encodes a grid of bitrate by height, scores each with VMAF, takes
  // the upper hull and cuts it where the quality stops improving. All of that was worked out
  // and stored, and none of it reached a screen — so the ladder arrived as an assertion, and
  // somebody who thought the top was too low had nothing to argue with but the number they
  // disagreed with.

  function measurement(): MeasurementView {
    return {
      run: { source_key: "1:film.mp4", codec: "h264" },
      points: [
        { bitrate_mbps: 35, height: 2160, actual_bps: 34_800_000, vmaf: 97.9 },
        { bitrate_mbps: 22, height: 2160, actual_bps: 21_700_000, vmaf: 96.4 },
        { bitrate_mbps: 12, height: 1440, actual_bps: 11_900_000, vmaf: 92.0 },
      ],
      selection: {
        rungs: [
          { bitrate_mbps: 22, height: 2160, vmaf: 96.4 },
          { bitrate_mbps: 12, height: 1440, vmaf: 92.0 },
        ],
        above_target: [{ bitrate_mbps: 35, height: 2160, vmaf: 97.9 }],
        hull: [],
      },
      ladder: null,
      notices: [],
    } as unknown as MeasurementView;
  }

  function measuredPlan() {
    return {
      ...preview("measured", MEASURED),
      codec: "h264",
      measurement_key: "1:film.mp4",
    } as unknown as LadderPreview;
  }

  it("shows every point that was encoded and what it scored", async () => {
    mockMeasureResult.mockResolvedValue(measurement());
    mockLadderPlan.mockResolvedValue(measuredPlan());
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    const block = await screen.findByTestId("measured-points");
    expect(block.querySelectorAll("tbody tr")).toHaveLength(3);
    // What the encoder was asked for and what it produced, side by side: they differ, and
    // where they differ a lot is where a rung costs more than its number says.
    expect(block.textContent).toContain("97.9");
    expect(block.textContent).toContain("34.8");
  });

  it("marks the points that became rungs apart from the ones that did not", async () => {
    mockMeasureResult.mockResolvedValue(measurement());
    mockLadderPlan.mockResolvedValue(measuredPlan());
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    const block = await screen.findByTestId("measured-points");
    const rows = [...block.querySelectorAll("tbody tr")];
    expect(rows.map((r) => r.getAttribute("data-chosen"))).toEqual(["no", "yes", "yes"]);
  });

  it("says what was dropped above the quality target", async () => {
    // T421. Otherwise a person hunts for the missing bitrate: the grid was measured up to 35
    // and the ladder tops out at 22, with nothing saying the rest went on purpose.
    mockMeasureResult.mockResolvedValue(measurement());
    mockLadderPlan.mockResolvedValue(measuredPlan());
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    const dropped = await screen.findByTestId("dropped-above");
    expect(dropped.textContent).toContain("35");
  });

  it("offers nothing to look into where nothing was measured", async () => {
    // A ladder from the formula measured nothing, and an empty fold saying so would be a
    // control that does nothing.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("provenance")).toBeTruthy());
    expect(screen.queryByTestId("measured-points")).toBeNull();
  });
});

describe("why each rung is what it is", () => {
  // T418, and the reason T199 was unticked. The core has been producing a list of reasons
  // per rung since milestone C; nothing drew them, and neither catalogue had a word for
  // any of them. A ladder built on a probe that failed looked exactly like one built on a
  // probe that worked.

  it("draws every reason a rung was given, not the first of them", async () => {
    const two = rung(1, 12, 1440, 92.0);
    two.reasons = ["step_down", "measured_optimum"];
    mockLadderPlan.mockResolvedValue(preview("measured", [rung(0, 22, 2160, 96.1), two]));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "ru");

    const why = await screen.findByTestId("why-1");
    // A rung is usually the result of more than one decision; showing one of them is
    // choosing which half of the answer to withhold.
    expect(why.querySelectorAll("li")).toHaveLength(2);
  });

  it("says how far down the step went, not merely that there was one", async () => {
    // A bare "a step down" is true of every rung but the top and explains none of them.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "ru");

    const why = await screen.findByTestId("why-1");
    expect(why.textContent).toContain("12");
  });

  it("puts the numbers of its own rung into the reason, not the ladder's", async () => {
    // The one way a column like this fails while looking right: every row saying the same
    // thing, because the numbers came from somewhere other than the row.
    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "ru");

    const first = await screen.findByTestId("why-0");
    const third = await screen.findByTestId("why-2");
    expect(first.textContent).not.toBe(third.textContent);
    expect(third.textContent).toContain("1080");
  });

  it("marks a borrowed measurement in the rung's own row", async () => {
    // T419, FR-145. Saying it once at the top of the set is not enough: a rung measured
    // here beside one lent from a neighbouring episode is exactly the case where the
    // difference matters, and a heading cannot say which is which.
    const lent = rung(1, 12, 1440, 92.0);
    lent.quality = { state: "borrowed", vmaf_x100: 9200 };
    mockLadderPlan.mockResolvedValue(preview("measured", [rung(0, 22, 2160, 96.1), lent]));
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "ru");

    await screen.findByTestId("rung-1");
    const mine = screen.getByTestId("rung-0").querySelector("[data-measured]");
    const theirs = screen.getByTestId("rung-1").querySelector("[data-measured]");
    expect(mine?.getAttribute("data-borrowed")).toBe("no");
    expect(theirs?.getAttribute("data-borrowed")).toBe("yes");
    expect(theirs?.textContent).not.toBe(mine?.textContent);
  });
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "known", factor_x100: 250, points: 40, seconds_per_point_x10: 415 },
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() => expect(screen.getByTestId("how-long")).toHaveTextContent("3 points"));
    expect(screen.getByTestId("how-long")).toHaveTextContent("of 12");
    // T423. "From your own measurements" alone is not something a person can check. The
    // seconds are: somebody who watched the last run knows whether forty seconds a point is
    // what they saw, and can disbelieve the estimate when it is not.
    const from = screen.getByTestId("estimate-from");
    expect(from).toHaveTextContent("40 points");
    expect(from).toHaveTextContent("41.5");
    expect(from).toHaveTextContent("2.5");
  });

  it("shows what the core noticed about this material before an hour is agreed to", async () => {
    // T422. `preview.notices` was carried across the boundary and never read. The probe can
    // fail, or run on a card it was not calibrated for, and either makes the top of the grid
    // a different kind of number — which is worth knowing before the hour, not after.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 600,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
      notices: [{ key: "NOTICE_PROBE_FAILED", params: {} }],
    } as unknown as MeasurePreview);
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    const said = await screen.findByTestId("offer-notices");
    expect(said.textContent).toContain(en.details.NOTICE_PROBE_FAILED);
  });

  it("can be asked what the estimate stands on", async () => {
    // The chunks and the anchor were both worked out and both dropped. They are the answer
    // to "why measure at all" and to "why that top and not another".
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 600,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
      notices: [],
    } as unknown as MeasurePreview);
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    const stands = await screen.findByTestId("stands-on");
    // 233 s, 590 s, 947 s — said in minutes, because that is the scale of a film.
    expect(stands.textContent).toContain("4, 10, 16");
    expect(stands.textContent).toContain("8 Mbit/s");
  });

  it("does not ask for a minute to measure a grid that is already measured", async () => {
    // T425. `Math.max(1, …)` is right while there is something left — "about 0 minutes"
    // reads as a mistake — and wrong when there is nothing: it offers a minute of work on a
    // finished grid, and whoever presses the button gets an instant finish and no idea why.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 12,
      about_seconds: 0,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
      notices: [],
    } as unknown as MeasurePreview);
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() =>
      expect(screen.getByTestId("how-long")).toHaveTextContent(en.ui.ladder.measureNothingLeft),
    );
    expect(screen.getByTestId("how-long")).not.toHaveTextContent("1 minute");
    // And the button goes with it: a control that does nothing teaches people the
    // application is broken.
    expect(screen.getByText(en.ui.ladder.measureStart)).toBeDisabled();
  });

  it("tells an unreadable store apart from an empty one", async () => {
    // T424. Both used to come out as "estimated from the model". They are not the same: with
    // an unreadable store there may be a hundred timed points, and the figure may be wrong by
    // a factor of three with nothing on screen to suggest doubting it.
    mockLadderPlan.mockResolvedValue(
      preview("formula", GUESSED, { code: "RUNGS_NOT_MEASURED", indexes: [0, 1] }),
    );
    mockMeasurePreview.mockResolvedValue({
      source_key: "1:film.mp4",
      points: 12,
      already_measured: 0,
      about_seconds: 600,
      chunk_starts: [233, 590, 947],
      anchor_mbps: 8,
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "not_asked" },
      notices: [],
    } as unknown as MeasurePreview);
    renderIn(<LadderScreen path="F:/films/film.mp4" />, "en");

    await waitFor(() =>
      expect(screen.getByTestId("estimate-from")).toHaveTextContent(en.ui.ladder.estimateNotAsked),
    );
    expect(screen.getByTestId("estimate-from")).not.toHaveTextContent(
      en.ui.ladder.estimateFromModel,
    );
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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

  it("keeps what the measurement said about itself beside the build button", async () => {
    // T416. A partial measurement is not a failure — the ladder is built from what came
    // out — but where the points were missing the optimum may never have been found, and
    // that is an argument against building from it. It used to reach `tracing::info!` and
    // stop there.
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
      notices: [],
    });
    renderIn(<LadderScreen path="F:/films/film.mp4" serverId="s1" />, "en");

    await waitFor(() => expect(screen.getByText(en.ui.ladder.measureStart)).toBeEnabled());
    fireEvent.click(screen.getByText(en.ui.ladder.measureStart));
    await waitFor(() => expect(mockMeasureStart).toHaveBeenCalled());
    await waitFor(() => expect(finish).not.toBeNull());

    mockLadderPlan.mockResolvedValue(preview("measured", MEASURED));
    finish?.({
      id: "task-1",
      state: "completed",
      error: null,
      notices: [{ key: "NOTICE_MEASUREMENT_PARTIAL", params: { measured: 9, total: 12 } }],
    });

    const said = await screen.findByTestId("measure-notices");
    expect(said.textContent).toContain("9");
    expect(said.textContent).toContain("12");
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
      encoder: { kind: "hardware", name: "h264_nvenc" },
      machine: { state: "nothing_timed_yet" },
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
