/**
 * T450, T451, T453 — where the working files go, and what stays behind when that changes.
 *
 * **The defect underneath.** A variant is one and a half to two gigabytes and used to be
 * written to the system's temporary directory: `C:` on Windows, often a memory-backed
 * filesystem on Linux. Films live on the big disk. So a build ended hours in, out of space
 * nobody agreed to spend.
 *
 * The core is stubbed; what is checked here is what a person can see and press.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { Settings } from "../../../shared/contract";
import { renderIn, ru } from "../../../test-utils";

let stored: Settings;
/** What the core answers about the tray. `null` stands for "would not say". */
let trayState: "installed" | "unavailable" | null = "installed";
const mockOpen = vi.fn<() => Promise<string | null>>();
const mockLeftovers = vi.fn<() => Promise<{ files: number; bytes: number }>>();

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => mockOpen() }));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      settingsGet: () => Promise.resolve(stored),
      settingsSet: (s: Settings) => {
        stored = s;
        return Promise.resolve(s);
      },
      workDirLeftovers: () => mockLeftovers(),
      trayState: () =>
        trayState === null ? Promise.reject(new Error("no answer")) : Promise.resolve(trayState),
      forgetEverything: vi.fn(),
    }),
  };
});

const { WorkDir } = await import("../WorkDir");
const { SettingsProvider } = await import("../../../app/settings");

function show() {
  return renderIn(
    <SettingsProvider>
      <WorkDir />
    </SettingsProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  stored = {
    viewer_activity_threshold_s: 30,
    geo_refine_outside: false,
    concurrent_heavy_tasks: 1,
    mascot: true,
    animations: true,
    language: null,
    theme: null,
    close_to_tray: true,
    tray_notice_seen: false,
    work_dir: null,
  };
  // Every stub gets an answer, not only the ones a given test reads: a stub without one
  // returns `undefined` and whatever calls it does `.then` on nothing. That was green here
  // and red in CI on 2026-08-28.
  mockOpen.mockResolvedValue(null);
  mockLeftovers.mockResolvedValue({ files: 0, bytes: 0 });
  trayState = "installed";
});

it("says the default is beside the source rather than naming a path", async () => {
  // The default is per film — it goes beside whichever source is being built — so there is
  // no one path to print. Printing one would be a lie about where the next build writes.
  show();
  await waitFor(() =>
    expect(screen.getByTestId("work-dir")).toHaveTextContent(ru.ui.appearance.workDirDefault),
  );
});

it("takes the folder a person chose", async () => {
  mockOpen.mockResolvedValue("E:/scratch");
  show();
  await waitFor(() => expect(screen.getByText(ru.ui.appearance.workDirPick)).toBeTruthy());

  fireEvent.click(screen.getByText(ru.ui.appearance.workDirPick));
  await waitFor(() => expect(screen.getByTestId("work-dir")).toHaveTextContent("E:/scratch"));
});

it("offers no way back while there is nothing to go back from", async () => {
  // A control that does nothing teaches people the application is broken.
  show();
  await waitFor(() => expect(screen.getByText(ru.ui.appearance.workDirPick)).toBeTruthy());
  expect(screen.queryByText(ru.ui.appearance.workDirReset)).toBeNull();
});

it("says what was left behind at the old path", async () => {
  // T453. Working files are swept after a variant is sent, so this is nothing nearly
  // always — and when it is not, it is gigabytes left by a build that was killed, under a
  // path the application stops looking at the moment this setting changes. Silently
  // forgotten gigabytes are the very fault T450 removes, only caused by us.
  stored = { ...stored, work_dir: "E:/scratch" };
  mockLeftovers.mockResolvedValue({ files: 2, bytes: 3_200_000_000 });
  mockOpen.mockResolvedValue("D:/elsewhere");
  show();
  await waitFor(() => expect(screen.getByTestId("work-dir")).toHaveTextContent("E:/scratch"));

  fireEvent.click(screen.getByText(ru.ui.appearance.workDirPick));

  const note = await screen.findByTestId("work-dir-left");
  expect(note.textContent).toContain("2");
  expect(note.textContent).toContain("3200");
});

it("says nothing where the old path is empty", async () => {
  // Which is the ordinary case. A note that always appears is a note nobody reads.
  stored = { ...stored, work_dir: "E:/scratch" };
  mockOpen.mockResolvedValue("D:/elsewhere");
  show();
  await waitFor(() => expect(screen.getByTestId("work-dir")).toHaveTextContent("E:/scratch"));

  fireEvent.click(screen.getByText(ru.ui.appearance.workDirPick));

  await waitFor(() => expect(screen.getByTestId("work-dir")).toHaveTextContent("D:/elsewhere"));
  expect(screen.queryByTestId("work-dir-left")).toBeNull();
});

// ---------- what the close button will do (T395, T400) ----------

it("says the window will be hidden where there is a tray", async () => {
  // The button does two different things depending on the desktop, and the difference costs
  // an afternoon of encoding in one direction or an application running unnoticed in the
  // other. Said before it is pressed rather than discovered by pressing it.
  const { CloseButton } = await import("../CloseButton");
  trayState = "installed";
  renderIn(<CloseButton />);
  const said = await screen.findByTestId("close-behaviour");
  expect(said.textContent).toBe(ru.ui.appearance.closeHides);
});

it("says the application will quit where there is not", async () => {
  const { CloseButton } = await import("../CloseButton");
  trayState = "unavailable";
  renderIn(<CloseButton />);
  const said = await screen.findByTestId("close-behaviour");
  expect(said.textContent).toBe(ru.ui.appearance.closeExits);
});

it("says it does not know rather than guessing at one of the two", async () => {
  // Not knowing is its own answer. Passing it off as either real one would be a promise
  // about what the button does, made without asking.
  const { CloseButton } = await import("../CloseButton");
  trayState = null;
  renderIn(<CloseButton />);
  const said = await screen.findByTestId("close-behaviour");
  expect(said.textContent).toBe(ru.ui.appearance.closeUnknown);
});
