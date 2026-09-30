/**
 * T658 — settings are saved one at a time, and a late answer does not bring back a choice.
 *
 * QA-24A №9: each change sent its own whole snapshot at once and took whichever answer came
 * back as the truth. Theme, then mascot off — the first save's answer arriving last put the
 * mascot back, and the next change sent that back to the core. Built on the QA round's own
 * probe (`qa24a/ui.test.tsx`), turned round to assert the wanted behaviour — and the core's
 * own corrections (T546) still have to reach the screen.
 */

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { renderIn } from "../../test-utils";
import type { AppError, Settings } from "../../shared/contract";

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  set: vi.fn(),
}));

vi.mock("../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../shared/ipc")>("../../shared/ipc");
  const { stubIpc } = await import("../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      settingsGet: () => mocks.get(),
      settingsSet: (s: Settings) => mocks.set(s),
    }),
  };
});

const { SettingsProvider, useSettings } = await import("../settings");

const base: Settings = {
  viewer_activity_threshold_s: 30,
  geo_refine_outside: false,
  concurrent_heavy_tasks: 1,
  mascot: true,
  animations: true,
  language: "ru",
  theme: "light",
  close_to_tray: true,
  tray_notice_seen: false,
  work_dir: null,
};

function deferred<T>() {
  let resolve!: (x: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function Panel() {
  const { settings: s, update, error } = useSettings();
  return (
    <>
      <output data-testid="snapshot">{JSON.stringify(s)}</output>
      <output data-testid="error">{error ? error.code : ""}</output>
      <button onClick={() => update({ theme: "dark" })}>dark</button>
      <button onClick={() => update({ mascot: false })}>mascot off</button>
      <button onClick={() => update({ animations: false })}>animations off</button>
      <button onClick={() => update({ concurrent_heavy_tasks: 20 })}>heavy 20</button>
    </>
  );
}

const shown = (): Settings => JSON.parse(screen.getByTestId("snapshot").textContent ?? "null");

async function draw() {
  renderIn(
    <SettingsProvider>
      <Panel />
    </SettingsProvider>,
  );
  await waitFor(() => expect(shown()).toEqual(base));
}

beforeEach(() => {
  mocks.get.mockReset();
  mocks.set.mockReset();
  mocks.get.mockResolvedValue(base);
});

it("a later change waits for the save in flight, and the late answer does not undo it", async () => {
  const first = deferred<Settings>();
  const second = deferred<Settings>();
  mocks.set.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await draw();

  fireEvent.click(screen.getByText("dark"));
  fireEvent.click(screen.getByText("mascot off"));

  // One save at a time: the mascot waits for the theme.
  expect(mocks.set).toHaveBeenCalledTimes(1);
  expect(mocks.set.mock.calls[0][0]).toMatchObject({ theme: "dark", mascot: true });
  // Both are on screen at once, all the same.
  expect(shown()).toMatchObject({ theme: "dark", mascot: false });

  // The theme's answer — which knows nothing of the mascot — comes back.
  await act(async () => first.resolve({ ...base, theme: "dark" }));
  expect(shown().mascot).toBe(false);
  // And the mascot goes out next, on top of that answer.
  expect(mocks.set).toHaveBeenCalledTimes(2);
  expect(mocks.set.mock.calls[1][0]).toMatchObject({ theme: "dark", mascot: false });

  await act(async () => second.resolve({ ...base, theme: "dark", mascot: false }));
  expect(shown()).toMatchObject({ theme: "dark", mascot: false });

  // The next change sends the choice as it stands, not the undone one.
  mocks.set.mockImplementation(async (s: Settings) => s);
  await act(async () => fireEvent.click(screen.getByText("animations off")));
  expect(mocks.set.mock.calls[2][0]).toMatchObject({
    theme: "dark",
    mascot: false,
    animations: false,
  });
});

it("changes made while a save is out go together in one save after it", async () => {
  const first = deferred<Settings>();
  mocks.set.mockReturnValueOnce(first.promise).mockImplementation(async (s: Settings) => s);
  await draw();

  fireEvent.click(screen.getByText("dark"));
  fireEvent.click(screen.getByText("mascot off"));
  fireEvent.click(screen.getByText("animations off"));
  expect(mocks.set).toHaveBeenCalledTimes(1);

  await act(async () => first.resolve({ ...base, theme: "dark" }));

  expect(mocks.set).toHaveBeenCalledTimes(2);
  expect(mocks.set.mock.calls[1][0]).toMatchObject({
    theme: "dark",
    mascot: false,
    animations: false,
  });
  await waitFor(() =>
    expect(shown()).toMatchObject({ theme: "dark", mascot: false, animations: false }),
  );
});

it("the core's correction still reaches the screen (T546)", async () => {
  mocks.set.mockImplementation(async (s: Settings) => ({ ...s, concurrent_heavy_tasks: 8 }));
  await draw();

  await act(async () => fireEvent.click(screen.getByText("heavy 20")));

  expect(mocks.set.mock.calls[0][0]).toMatchObject({ concurrent_heavy_tasks: 20 });
  await waitFor(() => expect(shown().concurrent_heavy_tasks).toBe(8));
});

it("a correction to one field stands while a newer change to another is laid over it", async () => {
  const first = deferred<Settings>();
  mocks.set.mockReturnValueOnce(first.promise).mockImplementation(async (s: Settings) => s);
  await draw();

  fireEvent.click(screen.getByText("heavy 20"));
  fireEvent.click(screen.getByText("mascot off"));

  // The core keeps 8 of the 20 asked for.
  await act(async () => first.resolve({ ...base, concurrent_heavy_tasks: 8 }));

  expect(mocks.set.mock.calls[1][0]).toMatchObject({ concurrent_heavy_tasks: 8, mascot: false });
  await waitFor(() => expect(shown()).toMatchObject({ concurrent_heavy_tasks: 8, mascot: false }));
});

it("a failed save is said, and what was changed after it is still saved", async () => {
  const first = deferred<Settings>();
  mocks.set.mockReturnValueOnce(first.promise).mockImplementation(async (s: Settings) => s);
  await draw();

  fireEvent.click(screen.getByText("dark"));
  fireEvent.click(screen.getByText("mascot off"));
  await act(async () => first.reject({ code: "INTERNAL" } as AppError));

  expect(mocks.set).toHaveBeenCalledTimes(2);
  expect(mocks.set.mock.calls[1][0]).toMatchObject({ theme: "dark", mascot: false });
  await waitFor(() => expect(shown()).toMatchObject({ theme: "dark", mascot: false }));
});

it("a failed save with nothing after it leaves the choice on screen and says why", async () => {
  mocks.set.mockRejectedValueOnce({ code: "INTERNAL" } as AppError);
  await draw();

  await act(async () => fireEvent.click(screen.getByText("mascot off")));

  expect(screen.getByTestId("error").textContent).toBe("INTERNAL");
  expect(shown().mascot).toBe(false);
  expect(mocks.set).toHaveBeenCalledTimes(1);
});
