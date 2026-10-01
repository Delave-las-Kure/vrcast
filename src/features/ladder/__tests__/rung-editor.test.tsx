/**
 * T673 — the rung editor on its own, now that it opens from a video's card.
 *
 * These cases used to be checked through the quality screen that hosted it. That screen is
 * gone; what the editor promises is not: a guess is never shown as a measurement, a hand edit
 * is recomputed by the core rather than patched, the freshest edit wins, and an objection
 * appears while the person is still editing.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn } from "../../../test-utils";
import type { LadderVerdict, Rung } from "../../../shared/contract";

const mockValidate = vi.fn<(...a: unknown[]) => Promise<LadderVerdict>>();
const mockRecompute = vi.fn<(...a: unknown[]) => Promise<Rung>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      ladderValidate: (...a: unknown[]) => mockValidate(...a),
      ladderRecomputeRung: (...a: unknown[]) => mockRecompute(...a),
    }),
  };
});

const { RungEditor } = await import("../RungEditor");

const SOURCE = {
  width: 3840,
  height: 2160,
  fps: 24,
  bitrate_bps: 60_000_000,
  heavier_codec: false,
  native_height: null,
};

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

function edited(index: number, mbps: number, height: number): Rung {
  return { ...rung(index, mbps, height, null), reasons: ["edited_by_hand"] };
}

const MEASURED = [rung(0, 22, 2160, 96.1), rung(1, 12, 1440, 92.0), rung(2, 6, 1080, 87.4)];

/** The editor with its rungs held in state, the way its host holds them. */
function Host({ initial, onToggle }: { initial: Rung[]; onToggle?: (i: number) => void }) {
  const [rungs, setRungs] = useState(initial);
  const [out, setOut] = useState<ReadonlySet<number>>(new Set());
  return (
    <RungEditor
      rungs={rungs}
      source={SOURCE}
      onChange={setRungs}
      left_out={out}
      onToggle={(i) => {
        onToggle?.(i);
        setOut((was) => {
          const next = new Set(was);
          if (!next.delete(i)) next.add(i);
          return next;
        });
      }}
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mockValidate.mockResolvedValue({ objections: [], not_buildable: null });
  mockRecompute.mockImplementation((index: unknown, bps: unknown) =>
    Promise.resolve(edited(index as number, (bps as number) / 1_000_000, 1080)),
  );
});

describe("what a rung is worth", () => {
  it("shows the measured quality of each rung and marks the ones without", async () => {
    renderIn(<Host initial={[rung(0, 22, 2160, 96.1), rung(1, 12, 1440, null)]} />, "en");
    expect(await screen.findByTestId("rung-0")).toHaveTextContent("96.10");
    expect(screen.getByTestId("rung-1")).toHaveTextContent(en.ui.ladder.notMeasured);
  });

  it("marks a borrowed measurement in the rung's own row", async () => {
    const lent = rung(1, 12, 1440, 92.0);
    lent.quality = { state: "borrowed", vmaf_x100: 9200 };
    renderIn(<Host initial={[rung(0, 22, 2160, 96.1), lent]} />, "en");
    await screen.findByTestId("rung-1");
    const mine = screen.getByTestId("rung-0").querySelector("[data-measured]");
    const theirs = screen.getByTestId("rung-1").querySelector("[data-measured]");
    expect(mine?.getAttribute("data-borrowed")).toBe("no");
    expect(theirs?.getAttribute("data-borrowed")).toBe("yes");
  });

  it("draws every reason a rung was given, with the rung's own numbers", async () => {
    const two = rung(1, 12, 1440, 92.0);
    two.reasons = ["step_down", "measured_optimum"];
    renderIn(<Host initial={[rung(0, 22, 2160, 96.1), two, rung(2, 6, 1080, 87.4)]} />, "ru");
    const why = await screen.findByTestId("why-1");
    expect(why.querySelectorAll("li")).toHaveLength(2);
    expect(why.textContent).toContain("12");
    expect(screen.getByTestId("why-2").textContent).toContain("1080");
  });
});

describe("editing a rung", () => {
  it("takes the whole rung back from the core, marked as typed by hand (T523)", async () => {
    mockRecompute.mockResolvedValue(edited(1, 3, 720));
    renderIn(<Host initial={MEASURED} />, "en");
    await waitFor(() => expect(screen.getByTestId("rung-1")).toHaveTextContent("92.00"));

    fireEvent.change(screen.getByLabelText(`${en.ui.ladder.columnBitrate} 2`), {
      target: { value: "3" },
    });

    await waitFor(() => expect(screen.getByTestId("rung-1")).toHaveTextContent("1280×720"));
    expect(mockRecompute).toHaveBeenCalledWith(1, 3_000_000, SOURCE);
    expect(screen.getByTestId("rung-1")).toHaveTextContent(en.ui.ladder.notMeasured);
    expect(screen.getByTestId("why-1").textContent).toContain("typed in by hand");
  });

  it("keeps the freshest edit when two answers arrive out of order (T523)", async () => {
    let resolveFirst: (r: Rung) => void = () => {};
    mockRecompute
      .mockImplementationOnce(() => new Promise<Rung>((resolve) => (resolveFirst = resolve)))
      .mockImplementationOnce(() => Promise.resolve(edited(1, 5, 1080)));
    renderIn(<Host initial={MEASURED} />, "en");
    await waitFor(() => expect(screen.getByTestId("rung-1")).toHaveTextContent("92.00"));

    const input = screen.getByLabelText(`${en.ui.ladder.columnBitrate} 2`);
    fireEvent.change(input, { target: { value: "9" } });
    fireEvent.change(input, { target: { value: "5" } });
    await waitFor(() => expect(screen.getByTestId("rung-1")).toHaveTextContent("1920×1080"));

    resolveFirst(edited(1, 9, 2160));
    await new Promise((r) => setTimeout(r, 10));
    expect(screen.getByTestId("rung-1")).toHaveTextContent("1920×1080");
    expect(screen.getByLabelText(`${en.ui.ladder.columnBitrate} 2`)).toHaveValue(5);
  });

  it("shows an objection as soon as an edit makes one (FR-044)", async () => {
    renderIn(<Host initial={MEASURED} />, "en");
    await screen.findByTestId("rung-0");
    mockValidate.mockResolvedValue({
      objections: [{ RungAboveSource: { index: 0, source_bps: 60_000_000 } }],
      not_buildable: null,
    });
    fireEvent.change(screen.getByLabelText(`${en.ui.ladder.columnBitrate} 1`), {
      target: { value: "90" },
    });
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("above the source"));
  });
});

describe("leaving a rung out", () => {
  it("checks only the rungs left in", async () => {
    renderIn(<Host initial={MEASURED} />, "en");
    await screen.findByTestId("rung-1");
    fireEvent.click(screen.getAllByRole("checkbox")[1]);

    await waitFor(() =>
      expect(screen.getByTestId("rung-1")).toHaveAttribute("data-left-out", "yes"),
    );
    await waitFor(() => {
      const calls = mockValidate.mock.calls;
      const last = calls[calls.length - 1]?.[0] as Rung[];
      expect(last.map((r) => r.index)).toEqual([0, 2]);
    });
  });
});
