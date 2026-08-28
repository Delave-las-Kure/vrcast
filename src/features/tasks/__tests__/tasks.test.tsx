/**
 * T413 — a progress event that says nothing about the stage changes nothing about it.
 *
 * **What T413 was written about, and what it turned out to be.** The task said the
 * stage of a running task was wiped when another task finished. It was — but the fault
 * was not here. Finishing makes the panel read the whole list again, and the records it
 * read carried `stage: null` because nothing in the core had ever written one. That is
 * T412, and the check that bites for it is `the_stage_a_task_reached_is_written_down`
 * in the core. A test of it here, against a stubbed list, would pass whatever the core
 * did — which is the kind of check this project spends its time deleting.
 *
 * **What is left is a rule worth holding.** `report_transfer` sends progress with no
 * stage at all, four times a second, and the panel used to copy that `null` over
 * whatever the task had last said. No task mixes the two today — the upload names its
 * stage only after the transfer — so this is not a live fault; it is a trap laid for
 * the first task that does, and Phase 30 exists to make tasks say more about
 * themselves, not less.
 *
 * The core is stubbed. What is checked here is what a person is left looking at.
 */

import { screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { Task } from "../../../shared/contract";
import { renderIn, ru } from "../../../test-utils";

let list: Task[] = [];
let progress: ((e: unknown) => void) | null = null;
let done: ((e: unknown) => void) | null = null;

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      tasksList: () => Promise.resolve(list),
      tasksOnClose: () => Promise.resolve([]),
      tasksReorder: vi.fn(),
      taskPause: vi.fn(),
      taskResume: vi.fn(),
      taskCancel: vi.fn(),
    },
    // Only its own handler is dropped on cleanup: a component unmounting from an earlier
    // test would otherwise wipe out the subscription this one has just made.
    onTaskProgress: async (handler: (e: unknown) => void) => {
      progress = handler;
      return () => {
        if (progress === handler) progress = null;
      };
    },
    onTaskDone: async (handler: (e: unknown) => void) => {
      done = handler;
      return () => {
        if (done === handler) done = null;
      };
    },
  };
});

const { TasksPanel } = await import("../TasksPanel");

function task(over: Partial<Task> = {}): Task {
  return {
    id: "t-1",
    kind: "upload",
    server_id: null,
    state: "running",
    progress: 0.4,
    stage: null,
    speed_bps: null,
    eta_s: null,
    resume_token: null,
    error: null,
    notices: [],
    queue_order: 1,
    created_at: "2026-08-28T10:00:00Z",
    updated_at: "2026-08-28T10:00:00Z",
    ...over,
  };
}

beforeEach(() => {
  list = [];
  progress = null;
  done = null;
});

it("keeps the stage when the next progress event carries none", async () => {
  list = [task({ stage: "STAGE_CHECKSUM" })];
  renderIn(<TasksPanel />);
  await screen.findByText(ru.details.STAGE_CHECKSUM);

  // Exactly the shape `report_transfer` sends: how fast, how long left, and nothing at
  // all about the stage.
  progress?.({
    id: "t-1",
    state: "running",
    progress: 0.5,
    stage: null,
    speed_bps: 12_500_000,
    eta_s: 90,
  });

  // Wait on the bar rather than on the speed: this is about the event having landed,
  // and the bar is the plainest evidence of that.
  await waitFor(() =>
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("50"),
  );
  expect(
    screen.queryByText(ru.details.STAGE_CHECKSUM),
    "an event that said nothing about the stage emptied the line under the bar",
  ).toBeTruthy();
});

it("replaces the stage when the task really does move on", async () => {
  // The half that keeps the other two honest: holding on to the old stage must not mean
  // holding on to it after the task has said a new one.
  list = [task({ kind: "build_ladder", stage: "STAGE_BUILDING_LADDER" })];
  renderIn(<TasksPanel />);
  await screen.findByText(ru.details.STAGE_BUILDING_LADDER);

  progress?.({
    id: "t-1",
    state: "running",
    progress: 0.6,
    stage: "STAGE_CUTTING_SEGMENTS",
    speed_bps: null,
    eta_s: null,
  });

  await screen.findByText(ru.details.STAGE_CUTTING_SEGMENTS);
  expect(screen.queryByText(ru.details.STAGE_BUILDING_LADDER)).toBeNull();
});

it("shows what a finished task had to say", async () => {
  // T416. The build worked out that three variants were already on the server and did not
  // make them again — which is why it took twenty minutes instead of two hours. That went
  // into `outcome.map(|_| ())` and nowhere else.
  list = [
    task({
      kind: "build_ladder",
      state: "completed",
      progress: 1,
      notices: [{ key: "NOTICE_VARIANTS_REUSED", params: { count: 3 } }],
    }),
  ];
  renderIn(<TasksPanel />);
  await screen.findByText(/3/);
  expect(screen.getByTestId("task-notices").textContent).toContain("3");
});

it("says nothing where a task had nothing to say", async () => {
  // A row that always appears is a row nobody reads.
  list = [task({ state: "completed", progress: 1 })];
  renderIn(<TasksPanel />);
  await screen.findByText(ru.ui.tasks.states.completed);
  expect(screen.queryByTestId("task-notices")).toBeNull();
});
