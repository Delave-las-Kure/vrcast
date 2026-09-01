/**
 * T443 — a season goes in with one press.
 *
 * The core is stubbed: what matters here is what a person can put in and what the core is
 * asked for. That the chain then runs without anybody watching is the core's own check
 * (`the_chain_is_in_the_core_and_not_on_a_screen`), and it has to be there — a screen test
 * cannot tell a chain that works from one that only works while the screen is mounted.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { QualityMeasureRequest } from "../../../shared/contract";
import { renderIn, ru } from "../../../test-utils";

const mockOpen = vi.fn<() => Promise<string[] | string | null>>();
const started: QualityMeasureRequest[] = [];
let refuseFrom: number | null = null;

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => mockOpen() }));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      qualityMeasureStart: (request: QualityMeasureRequest) => {
        if (refuseFrom !== null && started.length >= refuseFrom) {
          return Promise.reject({ code: "INTERNAL", details: [] });
        }
        started.push(request);
        return Promise.resolve(`task-${started.length}`);
      },
    }),
  };
});

vi.mock("../../servers/store", () => ({
  useActiveServer: () => ({ id: "s1", name: "home", state: "ready" }),
  isReady: () => true,
}));

const { BatchScreen } = await import("../BatchScreen");

beforeEach(() => {
  vi.clearAllMocks();
  started.length = 0;
  refuseFrom = null;
  mockOpen.mockResolvedValue([]);
});

async function put(files: string[]) {
  mockOpen.mockResolvedValue(files);
  renderIn(<BatchScreen />);
  await waitFor(() => expect(screen.getByText(ru.ui.batch.pick)).toBeTruthy());
  fireEvent.click(screen.getByText(ru.ui.batch.pick));
  await screen.findByTestId("batch-files");
}

it("takes several videos in one visit to the dialogue", async () => {
  // The whole point. The three other dialogues in this application set `multiple` to false by
  // hand, so a season of twelve meant sitting down twelve times.
  await put(["F:/films/s01e01.mkv", "F:/films/s01e02.mkv", "F:/films/s01e03.mkv"]);
  expect(screen.getByTestId("batch-count").textContent).toContain("3");
});

it("keeps what is already in when more are added", async () => {
  // A season split across two folders is two visits, and the second must not throw away the
  // first.
  await put(["F:/films/s01e01.mkv"]);
  mockOpen.mockResolvedValue(["F:/films/s01e02.mkv"]);
  fireEvent.click(screen.getByText(ru.ui.batch.pick));
  await waitFor(() => expect(screen.getByTestId("batch-count").textContent).toContain("2"));
});

it("takes the same file twice as once", async () => {
  // Otherwise the same film is measured twice and built over itself, and the second build
  // spends hours replacing what the first had just finished.
  await put(["F:/films/s01e01.mkv", "F:/films/s01e01.mkv"]);
  expect(screen.getByTestId("batch-count").textContent).toContain("1");
});

it("puts every film in as one batch, not one batch each", async () => {
  // **The fault that would make the stop button useless.** One identifier per press is what
  // "stop the whole batch" means; a fresh one per film would give ten batches of one, and the
  // button would stop a tenth of the work while saying it stopped all of it.
  await put(["F:/films/s01e01.mkv", "F:/films/s01e02.mkv"]);
  fireEvent.click(screen.getByTestId("batch-start"));
  await waitFor(() => expect(started).toHaveLength(2));

  const ids = new Set(started.map((r) => r.batch?.id));
  expect(ids.size).toBe(1);
  // And each names its own film, or the task list is a wall again.
  expect(started.map((r) => r.batch?.label)).toEqual(["s01e01", "s01e02"]);
});

it("asks the core to build each one, on the chosen server, in its own directory", async () => {
  await put(["F:/films/s01e01.mkv", "F:/films/s01e02.mkv"]);
  fireEvent.click(screen.getByTestId("batch-start"));
  await waitFor(() => expect(started).toHaveLength(2));

  expect(started[0].then_build).toEqual({ server_id: "s1", slug: "s01e01" });
  expect(started[1].then_build).toEqual({ server_id: "s1", slug: "s01e02" });
});

it("says what went in when one of them is refused", async () => {
  // **What stays in matters more than what failed.** A batch that stops on the eighth film
  // has seven measurements already running; telling somebody nothing started would send them
  // to cancel work that is under way.
  refuseFrom = 2;
  await put(["F:/films/a.mkv", "F:/films/b.mkv", "F:/films/c.mkv"]);
  fireEvent.click(screen.getByTestId("batch-start"));

  const said = await screen.findByTestId("batch-started");
  expect(said.textContent).toContain("2");
  expect(screen.getByRole("alert")).toBeTruthy();
});

it("offers nothing to start while nothing is in", async () => {
  renderIn(<BatchScreen />);
  await waitFor(() => expect(screen.getByText(ru.ui.batch.pick)).toBeTruthy());
  expect(screen.getByTestId("batch-start")).toBeDisabled();
});
