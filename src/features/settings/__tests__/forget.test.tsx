/**
 * T643 — "remove everything" is not offered while a task is going.
 *
 * The core is what refuses (`FORGET_TASKS_RUNNING`, a command still connecting included); what is
 * checked here is what a person sees: the button off and the reason next to it while a task is
 * going, on again once it is over, and the core's refusal put into words if it comes anyway.
 */

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppError, ForgetSeen, Task, WhatWent, WhatWouldGo } from "../../../shared/contract";
import { en, renderIn, ru } from "../../../test-utils";

let list: Task[] = [];
let done: ((e: unknown) => void) | null = null;
let progress: ((e: unknown) => void) | null = null;
const mockForget = vi.fn<(confirmed: boolean, seen: ForgetSeen) => Promise<WhatWent>>();

const first: WhatWouldGo = {
  data_dir: null,
  bytes: 0,
  servers: ["первый"],
  secrets: 1,
  locked_out: [],
};
let would: WhatWouldGo = first;
const mockPreview = vi.fn<() => Promise<WhatWouldGo>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      forgetPreview: () => mockPreview(),
      forgetEverything: (confirmed: boolean, seen: ForgetSeen) => mockForget(confirmed, seen),
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
  would = first;
  mockPreview.mockImplementation(() => Promise.resolve(would));
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
  await waitFor(() =>
    expect(mockForget).toHaveBeenCalledWith(true, { servers: ["первый"], locked_out: [] }),
  );
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

// ---------- T648 (QA-23 №2): the list is read again, and an old agreement does not carry over ----------

/** A deployment that has just made the only key to "первый": the profile is now managed_key. */
const lockedOut: WhatWouldGo = { ...first, locked_out: ["первый"] };

function endTask(id = "t-1") {
  act(() => done?.({ event: "done", id, state: "completed", error: null, notices: [] }));
}

it("the QA path: after a deployment ends the list is read again, the agreement withdrawn and the loss named", async () => {
  list = [task()];
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).toBeDisabled());
  expect(screen.queryByTestId("forget-locked-out")).toBeNull();
  expect(mockPreview).toHaveBeenCalledTimes(1);

  // The run kept its key and ended.
  would = lockedOut;
  list = [task({ state: "completed" })];
  await waitFor(() => expect(done).not.toBeNull());
  endTask();

  await waitFor(() => expect(screen.getByTestId("forget-locked-out")).toBeInTheDocument());
  expect(mockPreview).toHaveBeenCalledTimes(2);
  expect(screen.getByText(ru.ui.forget.lockedOut("первый"))).toBeInTheDocument();
  expect(screen.getByTestId("forget-agree")).not.toBeChecked();
  expect(screen.getByTestId("forget-changed")).toHaveTextContent(ru.ui.forget.changed);
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(mockForget).not.toHaveBeenCalled();

  // Agreed again, to the list that names the loss: now it goes, with that list.
  fireEvent.click(screen.getByTestId("forget-agree"));
  expect(screen.queryByTestId("forget-changed")).toBeNull();
  await waitFor(() => expect(button).not.toBeDisabled());
  fireEvent.click(button);
  await waitFor(() =>
    expect(mockForget).toHaveBeenCalledWith(true, {
      servers: ["первый"],
      locked_out: ["первый"],
    }),
  );
});

it("a task ending without changing the list keeps the agreement", async () => {
  list = [task()];
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).toBeDisabled());

  list = [task({ state: "completed" })];
  await waitFor(() => expect(done).not.toBeNull());
  endTask();
  await waitFor(() => expect(mockPreview).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(button).not.toBeDisabled());
  expect(screen.getByTestId("forget-agree")).toBeChecked();
  expect(screen.queryByTestId("forget-changed")).toBeNull();
});

it("the button is not let on by a fresh task list while the list of consequences is still being read", async () => {
  list = [task()];
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).toBeDisabled());

  // The re-read hangs: the task list says "nothing going", the consequences are not in yet.
  let answer: (w: WhatWouldGo) => void = () => {};
  mockPreview.mockImplementationOnce(() => new Promise((r) => (answer = r)));
  list = [task({ state: "completed" })];
  await waitFor(() => expect(done).not.toBeNull());
  endTask();
  await waitFor(() => expect(screen.queryByTestId("forget-tasks-running")).toBeNull());
  expect(button).toBeDisabled();
  expect(button).toHaveTextContent(ru.ui.forget.reading);

  act(() => answer(lockedOut));
  await waitFor(() => expect(screen.getByTestId("forget-locked-out")).toBeInTheDocument());
  expect(screen.getByTestId("forget-agree")).not.toBeChecked();
  expect(button).toBeDisabled();
});

it("a change caught by the read right before removing removes nothing", async () => {
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).not.toBeDisabled());

  // Changed with no task event seen by this screen (another window, a missed event).
  would = lockedOut;
  fireEvent.click(button);
  await waitFor(() => expect(screen.getByTestId("forget-locked-out")).toBeInTheDocument());
  expect(mockForget).not.toHaveBeenCalled();
  expect(screen.getByTestId("forget-agree")).not.toBeChecked();
  expect(screen.getByTestId("forget-changed")).toBeInTheDocument();
  expect(screen.queryByTestId("forget-done")).toBeNull();
});

it("the core's FORGET_PREVIEW_STALE is said, and the list read again", async () => {
  const stale: AppError = { code: "FORGET_PREVIEW_STALE", details: [] };
  mockForget.mockImplementationOnce(() => {
    // It changed between the last read and the removal: the core saw it, the screen did not.
    would = lockedOut;
    return Promise.reject(stale);
  });
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).not.toBeDisabled());
  fireEvent.click(button);

  await waitFor(() =>
    expect(screen.getByText(ru.errors.FORGET_PREVIEW_STALE.message)).toBeInTheDocument(),
  );
  await waitFor(() => expect(screen.getByTestId("forget-locked-out")).toBeInTheDocument());
  expect(screen.getByTestId("forget-agree")).not.toBeChecked();
  expect(screen.queryByTestId("forget-done")).toBeNull();
});

it("the order of the names is not a change", async () => {
  would = { ...first, servers: ["а", "б"] };
  renderIn(<Forget />);
  const button = await agree();
  await waitFor(() => expect(button).not.toBeDisabled());
  would = { ...first, servers: ["б", "а"] };
  fireEvent.click(button);
  await waitFor(() => expect(mockForget).toHaveBeenCalledTimes(1));
  expect(screen.queryByTestId("forget-changed")).toBeNull();
});

it("the T648 words say what happened, in both languages", () => {
  expect(ru.errors.FORGET_PREVIEW_STALE.message).toBe("Список того, что уйдёт, изменился");
  expect(en.errors.FORGET_PREVIEW_STALE.message).toBe("The list of what would go has changed");
  expect(ru.errors.FORGET_PREVIEW_STALE.hint).toMatch(/Ничего не удалено/);
  expect(en.errors.FORGET_PREVIEW_STALE.hint).toMatch(/Nothing was removed/);
  expect(ru.ui.forget.changed).toMatch(/согласие снято/);
  expect(en.ui.forget.changed).toMatch(/agreement has been withdrawn/);
});
