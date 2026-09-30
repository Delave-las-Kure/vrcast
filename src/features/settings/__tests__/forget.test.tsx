/**
 * T643 — "remove everything" is not offered while a task is going.
 *
 * The core is what refuses (`FORGET_TASKS_RUNNING`, a command still connecting included); what is
 * checked here is what a person sees: the button off and the reason next to it while a task is
 * going, on again once it is over, and the core's refusal put into words if it comes anyway.
 */

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppError, Task, WhatWent, WhatWouldGo } from "../../../shared/contract";
import { en, renderIn, ru } from "../../../test-utils";

let list: Task[] = [];
let done: ((e: unknown) => void) | null = null;
let progress: ((e: unknown) => void) | null = null;
const mockForget = vi.fn<(confirmed: boolean) => Promise<WhatWent>>();

const would: WhatWouldGo = {
  data_dir: null,
  bytes: 0,
  servers: ["первый"],
  secrets: 1,
  locked_out: [],
};

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      forgetPreview: () => Promise.resolve(would),
      forgetEverything: (confirmed: boolean) => mockForget(confirmed),
      tasksList: () => Promise.resolve(list),
    }),
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

const { Forget } = await import("../Forget");

function task(over: Partial<Task> = {}): Task {
  return {
    id: "t-1",
    kind: "deploy",
    can_resume: false,
    server_id: null,
    state: "running",
    progress: 0.4,
    stage: null,
    speed_bps: null,
    eta_s: null,
    resume_token: null,
    error: null,
    notices: [],
    batch: null,
    queue_order: 1,
    created_at: "2026-09-30T10:00:00Z",
    updated_at: "2026-09-30T10:00:00Z",
    result: null,
    ...over,
  };
}

async function agree() {
  const box = await screen.findByTestId("forget-agree");
  fireEvent.click(box);
  return screen.getByTestId("forget-do") as HTMLButtonElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  list = [];
  done = null;
  progress = null;
  mockForget.mockResolvedValue({ data_dir_removed: true, secrets_removed: 1, secrets_left: [] });
});

it("with no task going the button works once agreed", async () => {
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).not.toBeDisabled());
  expect(screen.queryByTestId("forget-tasks-running")).toBeNull();
  fireEvent.click(button);
  await waitFor(() => expect(mockForget).toHaveBeenCalledWith(true));
});

it.each(["running", "queued"] as const)(
  "a %s task turns the button off and says why",
  async (state) => {
    list = [task({ state })];
    renderIn(<Forget />);
    const button = await agree();
    await waitFor(() => expect(screen.getByTestId("forget-tasks-running")).toBeInTheDocument());
    expect(button).toBeDisabled();
    expect(screen.getByText(ru.ui.forget.tasksRunning)).toBeInTheDocument();
    fireEvent.click(button);
    expect(mockForget).not.toHaveBeenCalled();
  },
);

it("a paused task still held by the engine counts; a leftover row does not", async () => {
  list = [task({ state: "paused", can_resume: true })];
  const { unmount } = renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).toBeDisabled());
  unmount();

  list = [task({ state: "paused", can_resume: false }), task({ id: "t-2", state: "failed" })];
  renderIn(<Forget />);
  const again = await agree();
  await waitFor(() => expect(again).not.toBeDisabled());
});

it("the button comes back when the last task ends, and goes off when one starts", async () => {
  list = [task()];
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).toBeDisabled());

  list = [task({ state: "completed" })];
  await waitFor(() => expect(done).not.toBeNull());
  act(() => done?.({ event: "done", id: "t-1", state: "completed", error: null, notices: [] }));
  await waitFor(() => expect(button).not.toBeDisabled());
  expect(screen.queryByTestId("forget-tasks-running")).toBeNull();

  act(() =>
    progress?.({
      event: "progress",
      id: "t-9",
      state: "running",
      progress: 0.1,
      stage: null,
      speed_bps: null,
      eta_s: null,
    }),
  );
  await waitFor(() => expect(button).toBeDisabled());
});

it("the core's refusal is put into words, in both languages", async () => {
  const refused: AppError = { code: "FORGET_TASKS_RUNNING", details: [] };
  mockForget.mockRejectedValue(refused);
  const { unmount } = renderIn(<Forget />);
  fireEvent.click(await agree());
  await waitFor(() =>
    expect(screen.getByText(ru.errors.FORGET_TASKS_RUNNING.message)).toBeInTheDocument(),
  );
  expect(screen.getByText(ru.errors.FORGET_TASKS_RUNNING.hint as string)).toBeInTheDocument();
  expect(screen.queryByTestId("forget-done")).toBeNull();
  unmount();

  renderIn(<Forget />, "en");
  fireEvent.click(await agree());
  await waitFor(() =>
    expect(screen.getByText(en.errors.FORGET_TASKS_RUNNING.message)).toBeInTheDocument(),
  );
  expect(screen.getByText(en.errors.FORGET_TASKS_RUNNING.hint as string)).toBeInTheDocument();
});

it("the words say what to do, in both languages", () => {
  expect(ru.errors.FORGET_TASKS_RUNNING.message).toBe("Сначала остановите задачи");
  expect(en.errors.FORGET_TASKS_RUNNING.message).toBe("Stop the tasks first");
  expect(ru.ui.forget.tasksRunning).toMatch(/остановите задачи/);
  expect(en.ui.forget.tasksRunning).toMatch(/Stop the tasks first/);
  expect(ru.errors.FORGET_IN_PROGRESS.hint).toMatch(/не запускаются/);
  expect(en.errors.FORGET_IN_PROGRESS.hint).toMatch(/no new task starts/);
});
