/**
 * T427, T428 — taking another film's measurement, and getting back out of it.
 *
 * **A capability with no way in.** All three commands were registered in the core, written
 * into the contract, and called from nowhere. So the second episode of a season — whose
 * ladder the first episode's measurement answers exactly — had to be measured again from
 * scratch: half an hour per episode, twelve times a season, for an answer already stored.
 *
 * The core is stubbed. What is checked here is what a person can reach.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { StoredMeasurement } from "../../../shared/contract";
import { renderIn, ru } from "../../../test-utils";

let stored: StoredMeasurement[] = [];
const mockReuse = vi.fn<(from: string, req: unknown) => Promise<unknown>>();
const mockForget = vi.fn<(key: string, codec: string) => Promise<void>>();
const changed = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      qualityMeasurements: () => Promise.resolve(stored),
      qualityMeasureReuse: (from: string, req: unknown) => mockReuse(from, req),
      qualityMeasureForget: (key: string, codec: string) => mockForget(key, codec),
    }),
  };
});

const { Borrow } = await import("../Borrow");

function measurement(over: Partial<StoredMeasurement> = {}): StoredMeasurement {
  return {
    source_key: "1:s01e01.mkv",
    codec: "h264",
    source_path: "F:/films/s01e01.mkv",
    width: 3840,
    height: 2160,
    fps: 24,
    source_bitrate_bps: 60_000_000,
    heavier_codec: false,
    native_height: 1080,
    anchor_mbps: 16,
    chunk_starts: [233, 590, 947],
    chunk_s: 10,
    borrowed_from: null,
    donor_anchor_mbps: null,
    material: null,
    ...over,
  };
}

function show(over: Partial<Parameters<typeof Borrow>[0]> = {}) {
  return renderIn(
    <Borrow
      path="F:/films/s01e02.mkv"
      borrowedFrom={null}
      measuredHere={false}
      sourceKey={null}
      codec="h264"
      onChanged={changed}
      {...over}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  stored = [];
  mockReuse.mockResolvedValue({});
  mockForget.mockResolvedValue(undefined);
});

it("offers the measurements already taken", async () => {
  stored = [measurement()];
  show();
  await screen.findByTestId("donors");
  expect(screen.getByTestId("donors").textContent).toContain("s01e01");
});

it("hands the core the donor and this film, and lets it decide", async () => {
  // The core refuses when the material is not the same, and says which field differed
  // (T431). This screen never second-guesses that — it asks.
  stored = [measurement()];
  show();
  await screen.findByTestId("donors");

  fireEvent.click(screen.getByText(ru.ui.ladder.borrowTake));
  await waitFor(() => expect(mockReuse).toHaveBeenCalled());
  expect(mockReuse.mock.calls[0][0]).toBe("1:s01e01.mkv");
  expect(mockReuse.mock.calls[0][1]).toEqual({ path: "F:/films/s01e02.mkv", codec: "h264" });
  expect(changed).toHaveBeenCalled();
});

it("does not offer a film its own measurement", async () => {
  // Borrowing from yourself is a no-op that looks like a choice.
  stored = [measurement({ source_key: "2:s01e02.mkv", source_path: "F:/films/s01e02.mkv" })];
  show({ sourceKey: "2:s01e02.mkv", measuredHere: true });
  await waitFor(() => expect(screen.queryByTestId("donors")).toBeNull());
});

it("does not offer a measurement that is itself borrowed", async () => {
  // The chain would work — the core follows it to the true donor (T429) — but a list of
  // copies of one measurement hides how few real ones there are.
  stored = [measurement({ borrowed_from: "F:/films/s01e01.mkv" })];
  show();
  await screen.findByTestId("no-donors");
});

it("says where borrowed rungs came from", async () => {
  show({ borrowedFrom: "F:/films/s01e01.mkv", sourceKey: "2:s01e02.mkv" });
  const said = await screen.findByTestId("borrowed-from");
  expect(said.textContent).toContain("s01e01");
});

it("offers a way out of a borrowed measurement", async () => {
  // T428. Until now the offer to measure vanished the moment any measurement was found,
  // borrowed or not — so a loan was a decision made once and for good.
  show({ borrowedFrom: "F:/films/s01e01.mkv", sourceKey: "2:s01e02.mkv" });
  const out = await screen.findByTestId("forget");
  expect(out.textContent).toBe(ru.ui.ladder.forgetBorrowed);

  fireEvent.click(out);
  await waitFor(() => expect(mockForget).toHaveBeenCalledWith("2:s01e02.mkv", "h264"));
  expect(changed).toHaveBeenCalled();
});

it("offers a way out of one's own measurement too", async () => {
  // A measurement of one's own can be wrong: the file was re-encoded, or the probe ran on a
  // card it was not calibrated for. Throwing it away is how it is taken again.
  show({ measuredHere: true, sourceKey: "2:s01e02.mkv" });
  const out = await screen.findByTestId("forget");
  expect(out.textContent).toBe(ru.ui.ladder.forgetMeasured);
});

it("offers nothing to forget where there is nothing", async () => {
  // A control that does nothing teaches people the application is broken.
  show();
  await waitFor(() => expect(screen.getByTestId("borrow")).toBeTruthy());
  expect(screen.queryByTestId("forget")).toBeNull();
});

it("does not offer to borrow over a measurement this film already has", async () => {
  // Handing somebody else's measurement to a film that has its own would overwrite an
  // answer without saying so. The way out is above; that is the order.
  stored = [measurement()];
  show({ measuredHere: true, sourceKey: "2:s01e02.mkv" });
  await screen.findByTestId("forget");
  expect(screen.queryByTestId("donors")).toBeNull();
});
