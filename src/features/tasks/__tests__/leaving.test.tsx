/**
 * T400 — leaving through the tray menu costs a question first (FR-086).
 *
 * ⚠ **What this was written about was a live hole, not a missing test.** `tray/mod.rs` had
 * `QUIT => app.exit(0)`: the menu item ended the application on the spot. The close button
 * had been taught to warn (T394, T395) and the item beside it had not, and those two are the
 * only ways out. Somebody with a thirty-gigabyte upload running chose "Exit" and lost it
 * without a word.
 *
 * The half that cannot be checked from here is that the item is visible and that clicking it
 * arrives — no call answers either on Linux (R-35). What can be checked is everything after
 * the question is asked, and that nothing asks it when nobody did.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { TaskOnClose } from "../../../shared/contract";
import { fill } from "../../../shared/i18n/render";
import { renderIn, ru } from "../../../test-utils";

const onClose = vi.fn<() => Promise<TaskOnClose[]>>();
const exit = vi.fn<() => Promise<void>>();
let ask: (() => void) | null = null;

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      tasksOnClose: () => onClose(),
      appExit: () => exit(),
    }),
    onAppQuitRequested: async (handler: () => void) => {
      ask = handler;
      return () => {
        if (ask === handler) ask = null;
      };
    },
  };
});

const { LeaveConfirm } = await import("../LeaveConfirm");

const carriesOn: TaskOnClose = {
  id: "a",
  kind: "upload",
  progress: 0.6,
  outcome: "resumes",
  explanation: { key: "ON_CLOSE_RESUMES_FROM", params: { percent: 60 } },
};
const fromTheStart: TaskOnClose = {
  id: "b",
  kind: "convert",
  progress: 0.4,
  outcome: "restarts",
  explanation: { key: "ON_CLOSE_RESTARTS_LOSING", params: { percent: 40 } },
};

beforeEach(() => {
  onClose.mockReset();
  exit.mockReset();
  onClose.mockResolvedValue([carriesOn, fromTheStart]);
  ask = null;
});

/** Put the question the way the tray menu does, and wait for the answer to be drawn. */
async function trayExitPressed() {
  await waitFor(() => expect(ask).not.toBeNull());
  ask?.();
}

it("shows nothing, and asks the core nothing, until somebody chooses to leave", async () => {
  // **The half of T400 that is about the close button.** Minimising to the tray goes nowhere
  // near here — it is handled in the core and never reaches the interface — so `tasks_on_close`
  // must not be called by this screen merely existing. Kept fresh in the background it would
  // also be a list read at some earlier moment shown as the state of things now.
  renderIn(<LeaveConfirm />);
  await waitFor(() => expect(ask).not.toBeNull());

  expect(screen.queryByTestId("leave-confirm")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

it("names what happens to each task rather than saying tasks are running", async () => {
  // FR-086 in as many words: a general "tasks are running, close anyway?" is not enough,
  // because it gives nothing to decide on.
  renderIn(<LeaveConfirm />);
  await trayExitPressed();

  await screen.findByTestId("leave-confirm");
  expect(screen.getByText(ru.ui.tasks.closeLosing)).toBeInTheDocument();
  expect(
    screen.getByText(fill(ru.details.ON_CLOSE_RESTARTS_LOSING, { percent: 40 }, ru, "ru")),
  ).toBeInTheDocument();
  expect(
    screen.getByText(fill(ru.details.ON_CLOSE_RESUMES_FROM, { percent: 60 }, ru, "ru")),
  ).toBeInTheDocument();
});

it("leaves only when told to, and not by being asked", async () => {
  renderIn(<LeaveConfirm />);
  await trayExitPressed();
  await screen.findByTestId("leave-confirm");

  // The question is on the screen and nothing has ended. This is the whole difference
  // between what the menu did before and what it does now.
  expect(exit).not.toHaveBeenCalled();

  fireEvent.click(screen.getByTestId("leave-yes"));
  await waitFor(() => expect(exit).toHaveBeenCalledTimes(1));
});

it("staying is an answer, and it ends the question rather than the application", async () => {
  // A dialog whose "no" merely hides it while the application exits anyway is worse than no
  // dialog: it takes the decision and reports the opposite of it.
  renderIn(<LeaveConfirm />);
  await trayExitPressed();
  await screen.findByTestId("leave-confirm");

  fireEvent.click(screen.getByTestId("leave-no"));
  await waitFor(() => expect(screen.queryByTestId("leave-confirm")).toBeNull());
  expect(exit).not.toHaveBeenCalled();
});

it("says the consequences are unknown rather than showing an empty list", async () => {
  // An empty list reads as "nothing to lose" — the one wrong thing to say here, because the
  // core asked the question precisely on finding something.
  onClose.mockRejectedValue(new Error("the core would not answer"));
  renderIn(<LeaveConfirm />);
  await trayExitPressed();

  await screen.findByTestId("leave-unknown");
  expect(screen.getByText(ru.ui.tasks.leaveUnknown)).toBeInTheDocument();
  // And it is still a question: not knowing is not a reason to decide for somebody.
  expect(screen.getByTestId("leave-yes")).toBeInTheDocument();
  expect(screen.getByTestId("leave-no")).toBeInTheDocument();
  expect(exit).not.toHaveBeenCalled();
});
