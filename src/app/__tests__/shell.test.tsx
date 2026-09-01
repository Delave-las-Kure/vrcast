/**
 * T028 — tests for the interface shell.
 *
 * The core is replaced here: a test of the interface must not need a live application,
 * a server or a database. What is checked is behaviour on screen — that the sections
 * are there, that the unfinished ones are marked honestly, that an error from the core
 * reaches a person intact, and that both languages actually work.
 *
 * Assertions read the words out of the catalogue rather than repeating them. A test
 * that repeats the text passes when the wording drifts and fails when it is corrected,
 * which is exactly backwards.
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, Settings, Task, TaskOnClose } from "../../shared/contract";
import { en, renderIn, ru } from "../../test-utils";
import { fill } from "../../shared/i18n/render";

// The replacement has to be declared before the code under test is imported.
const mockTasksList = vi.fn<() => Promise<Task[]>>();
const mockTasksOnClose = vi.fn<() => Promise<TaskOnClose[]>>();
const mockTasksReorder = vi.fn<(ids: string[]) => Promise<number>>();
const mockAppVersions = vi.fn();
const mockSettingsGet = vi.fn<() => Promise<Settings>>();
const mockSettingsSet = vi.fn<(s: unknown) => Promise<Settings>>();

/** What the core answers about settings until a test says otherwise. */
const SETTINGS: Settings = {
  viewer_activity_threshold_s: 30,
  geo_refine_outside: false,
  concurrent_heavy_tasks: 1,
  mascot: true,
  animations: true,
  language: null,
  theme: null,
  work_dir: null,
};

vi.mock("../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../shared/ipc")>("../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      appVersions: () => mockAppVersions(),
      tasksList: () => mockTasksList(),
      taskGet: vi.fn(),
      taskCancel: vi.fn(),
      taskPause: vi.fn(),
      taskResume: vi.fn(),
      // Wrapped rather than referenced: the mock factory is hoisted to the top of the file,
      // and a direct reference would be evaluated before the variable exists.
      tasksReorder: (ids: string[]) => mockTasksReorder(ids),
      settingsGet: () => mockSettingsGet(),
      settingsSet: (s: unknown) => mockSettingsSet(s),
      tasksQueueOrder: vi.fn(async () => []),
      // Returns a list rather than nothing: the real command always hands back a
      // list, and a replacement returning undefined would test behaviour that
      // does not happen.
      tasksOnClose: () => mockTasksOnClose(),
      serverProbeFingerprint: vi.fn(),
    }),
    onTaskProgress: vi.fn(async () => () => {}),
    onTaskDone: vi.fn(async () => () => {}),
    onTaskNotify: vi.fn(async () => () => {}),
    onLibraryChanged: vi.fn(async () => () => {}),
    onViewersUpdate: vi.fn(async () => () => {}),
  };
});

const { default: App } = await import("../App");
const { ThemeProvider } = await import("../theme");
const { SettingsProvider } = await import("../settings");
const { ErrorNotice } = await import("../../features/shared/ErrorNotice");

function makeTask(over: Partial<Task> = {}): Task {
  return {
    id: "t1",
    kind: "upload",
    server_id: null,
    state: "running",
    progress: 0.42,
    stage: "STAGE_CHECKSUM",
    speed_bps: 2_500_000,
    eta_s: 900,
    resume_token: null,
    error: null,
    notices: [],
    batch: null,
    queue_order: 1,
    created_at: "2026-08-25T10:00:00Z",
    updated_at: "2026-08-25T10:05:00Z",
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockAppVersions.mockResolvedValue({ app: "0.1.0", server: null, schema: 2 });
  mockSettingsGet.mockResolvedValue(SETTINGS);
  mockSettingsSet.mockImplementation(async (s) => s as Settings);
  mockTasksList.mockResolvedValue([]);
  mockTasksOnClose.mockResolvedValue([]);
  mockTasksReorder.mockResolvedValue(0);
  document.documentElement.dataset.theme = "";
  localStorage.clear();
  // The application picks its language from the system when the core has nothing stored,
  // and `navigator.language` differs between a developer's machine and CI. Pinning it
  // keeps a failure here about the code rather than about where it ran.
  //
  // Pinned **through the core** (T324): that is now the one place the choice lives, and a
  // test that set `localStorage` would be testing a store the application no longer reads.
  mockSettingsGet.mockResolvedValue({ ...SETTINGS, language: "ru" });
  // HashRouter keeps the address in the window itself and it survives unmounting:
  // without a reset the next test opens on whatever section the last one left.
  window.location.hash = "#/";
});

describe("the shell", () => {
  it("shows every section of the application", async () => {
    renderIn(<App />);
    // Looked for inside the menu: the name of the open section also appears as a
    // heading, and a search across the page would find two matches.
    const nav = await screen.findByRole("navigation", { name: ru.ui.sidebar.sections });
    for (const label of Object.values(ru.ui.sections)) {
      // Not an exact match any more: the three that are one job done in order carry their
      // number in front of the name.
      expect(within(nav).getByText(label, { exact: false })).toBeInTheDocument();
    }
  });

  it("numbers the three steps of one job in the order the work happens", async () => {
    // Preparation, then cutting into qualities, then sending. The menu used to list sending
    // before cutting, so somebody following it down the page was led the wrong way round.
    renderIn(<App />);
    const nav = await screen.findByRole("navigation", { name: ru.ui.sidebar.sections });
    const numbered = within(nav)
      .getAllByRole("link")
      .map((link) => link.textContent?.trim() ?? "")
      .filter((text) => /^\d\./.test(text));

    expect(numbered).toEqual([
      `1. ${ru.ui.sections.convert}`,
      `2. ${ru.ui.sections.ladder}`,
      `3. ${ru.ui.sections.upload}`,
    ]);
  });

  it("shows the application version when the core returned one", async () => {
    renderIn(<App />);
    // Built from the catalogue's own template: the sentence may be reworded, the fact that
    // the version is shown may not.
    expect(
      await screen.findByText(fill(ru.ui.sidebar.version, { version: "0.1.0" }, ru, "ru")),
    ).toBeInTheDocument();
  });

  it("does not fall over when the version could not be had", async () => {
    // The version is decoration. Its absence is no reason for a full-screen error.
    mockAppVersions.mockRejectedValue(new Error("core unavailable"));
    renderIn(<App />);
    // Looked for inside the menu: the name of the open section is also its heading, and a
    // search across the page finds two.
    const nav = await screen.findByRole("navigation", { name: ru.ui.sidebar.sections });
    expect(within(nav).getByText(ru.ui.sections.tasks)).toBeInTheDocument();
    expect(
      screen.queryByText(fill(ru.ui.sidebar.version, { version: "0.1.0" }, ru, "ru")),
    ).not.toBeInTheDocument();
  });

  it("opens the task section by default", async () => {
    renderIn(<App />);
    expect(await screen.findByText(ru.ui.tasks.empty)).toBeInTheDocument();
  });
});

describe("language", () => {
  it("shows the interface in English when English is chosen", async () => {
    mockSettingsGet.mockResolvedValue({ ...SETTINGS, language: "en" });
    renderIn(<App />, "en");

    const nav = await screen.findByRole("navigation", { name: en.ui.sidebar.sections });
    expect(within(nav).getByText(en.ui.sections.tasks)).toBeInTheDocument();
    expect(await screen.findByText(en.ui.tasks.empty)).toBeInTheDocument();
    // And nothing of the other language is left over on the screen.
    expect(screen.queryByText(ru.ui.tasks.empty)).not.toBeInTheDocument();
  });

  it("changes the whole screen when the language is switched", async () => {
    // The point of the feature: switching is immediate and total, not a reload with
    // half the screen left behind.
    renderIn(<App />);
    expect(await screen.findByText(ru.ui.tasks.empty)).toBeInTheDocument();

    const chooser = await screen.findByLabelText(ru.ui.common.language);
    fireEvent.change(chooser, { target: { value: "en" } });

    expect(await screen.findByText(en.ui.tasks.empty)).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.tasks.empty)).not.toBeInTheDocument();
  });

  it("remembers the choice for the next start", async () => {
    renderIn(<App />);
    const chooser = await screen.findByLabelText(ru.ui.common.language);
    fireEvent.change(chooser, { target: { value: "en" } });

    // Handed to the core, which is the one place the choice lives (T324). It used to go
    // into `localStorage` while the core kept a `language` field nobody read — and two
    // stores of one choice diverge silently, leaving a person looking at one language
    // while the settings claim another.
    await waitFor(() => {
      expect(mockSettingsSet).toHaveBeenCalledWith(expect.objectContaining({ language: "en" }));
    });
  });

  it("names each language in itself", async () => {
    // Someone who has landed in a language they cannot read must still be able to
    // find their own. Translating the names of languages would hide it from them.
    renderIn(<App />);
    const chooser = await screen.findByLabelText(ru.ui.common.language);
    expect(within(chooser).getByText("English")).toBeInTheDocument();
    expect(within(chooser).getByText("Русский")).toBeInTheDocument();
  });
});

describe("the task list", () => {
  it("shows a task with its state and its progress", async () => {
    mockTasksList.mockResolvedValue([makeTask()]);
    renderIn(<App />);

    expect(await screen.findByText(ru.ui.tasks.kinds.upload)).toBeInTheDocument();
    expect(await screen.findByText(ru.ui.tasks.states.running)).toBeInTheDocument();

    const bar = await screen.findByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "42");
  });

  it("shows the stage in words rather than as a code", async () => {
    mockTasksList.mockResolvedValue([makeTask({ stage: "STAGE_CHECKSUM" })]);
    renderIn(<App />);

    expect(await screen.findByText(ru.details.STAGE_CHECKSUM)).toBeInTheDocument();
    expect(screen.queryByText("STAGE_CHECKSUM")).not.toBeInTheDocument();
  });

  it("puts the figures into human terms", async () => {
    mockTasksList.mockResolvedValue([makeTask({ speed_bps: 2_500_000, eta_s: 5400 })]);
    renderIn(<App />);

    // 2 500 000 bytes/s is 20 Mbit/s; showing bytes to a person is pointless.
    expect(await screen.findByText("20,0 Мбит/с")).toBeInTheDocument();
    expect(await screen.findByText(/осталось ~1 ч 30 мин/)).toBeInTheDocument();
  });

  it("offers to pause a running task and resume a paused one", async () => {
    mockTasksList.mockResolvedValue([
      makeTask({ id: "a", state: "running" }),
      makeTask({ id: "b", state: "paused" }),
    ]);
    renderIn(<App />);

    expect(await screen.findByText(ru.ui.tasks.pause)).toBeInTheDocument();
    expect(await screen.findByText(ru.ui.tasks.resume)).toBeInTheDocument();
  });

  it("offers no actions on a finished task", async () => {
    mockTasksList.mockResolvedValue([makeTask({ state: "completed", progress: 1 })]);
    renderIn(<App />);

    expect(await screen.findByText(ru.ui.tasks.states.completed)).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.tasks.stop)).not.toBeInTheDocument();
    expect(screen.queryByText(ru.ui.tasks.pause)).not.toBeInTheDocument();
  });

  it("explains a failed task in the language in use", async () => {
    // The error was recorded as codes, so a task that failed a week ago still
    // explains itself in whatever language is chosen today.
    mockTasksList.mockResolvedValue([
      makeTask({
        state: "failed",
        error: { code: "CHECKSUM_MISMATCH", details: [{ key: "UPLOAD_CHECKSUM_MISMATCH" }] },
      }),
    ]);
    renderIn(<App />);

    expect(await screen.findByText(ru.details.UPLOAD_CHECKSUM_MISMATCH)).toBeInTheDocument();
  });

  it("shows the core's error when the list could not be read", async () => {
    const err: AppError = { code: "STORAGE_FAILED" };
    mockTasksList.mockRejectedValue(err);
    renderIn(<App />);

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(await screen.findByText(ru.errors.STORAGE_FAILED.message)).toBeInTheDocument();
    expect(await screen.findByText(ru.errors.STORAGE_FAILED.hint)).toBeInTheDocument();
  });
});

describe("showing errors", () => {
  it("words the core's code from the catalogue, with its particulars", () => {
    // The interface does not invent phrases: one catalogue means the same trouble
    // is explained the same way on every screen (FR-105).
    const err: AppError = {
      code: "HOST_KEY_CHANGED",
      cause: "expected SHA256:aaa, got SHA256:bbb",
    };
    renderIn(<ErrorNotice error={err} />);

    expect(screen.getByText(ru.errors.HOST_KEY_CHANGED.message)).toBeInTheDocument();
    expect(screen.getByText(ru.errors.HOST_KEY_CHANGED.hint)).toBeInTheDocument();
    // The particulars are shown as they arrived: they can be searched for.
    expect(screen.getByText(err.cause!)).toBeInTheDocument();
  });

  it("words the same error in English when English is chosen", () => {
    const err: AppError = { code: "HOST_KEY_CHANGED" };
    renderIn(<ErrorNotice error={err} />, "en");

    expect(screen.getByText(en.errors.HOST_KEY_CHANGED.message)).toBeInTheDocument();
    expect(screen.queryByText(ru.errors.HOST_KEY_CHANGED.message)).not.toBeInTheDocument();
  });

  it("says the specific thing the core named, not just the general code", () => {
    const err: AppError = {
      code: "INVALID_INPUT",
      details: [{ key: "PROFILE_PORT_RANGE" }],
    };
    renderIn(<ErrorNotice error={err} />);

    expect(screen.getByText(ru.details.PROFILE_PORT_RANGE)).toBeInTheDocument();
  });
});

describe("appearance", () => {
  it("follows the system by default", async () => {
    renderIn(
      <SettingsProvider>
        <ThemeProvider>
          <span>content</span>
        </ThemeProvider>
      </SettingsProvider>,
    );
    await waitFor(() => {
      expect(["light", "dark"]).toContain(document.documentElement.dataset.theme);
    });
  });

  it("remembers the choice between starts", async () => {
    mockSettingsGet.mockResolvedValue({ ...SETTINGS, theme: "dark" });
    renderIn(
      <SettingsProvider>
        <ThemeProvider>
          <span>content</span>
        </ThemeProvider>
      </SettingsProvider>,
    );
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("dark");
    });
  });

  it("follows the system when the system changes under it", async () => {
    // Not only at startup. Somebody switches their system theme in the evening, and an
    // application that only notices at the next start glares white in a dark room.
    let listener: (() => void) | null = null;
    let dark = false;
    // `matches` is a getter, not a value: the provider asks it when the event arrives rather
    // than when it was made, and a frozen copy would answer about yesterday's system.
    vi.stubGlobal("matchMedia", (query: string) => ({
      get matches() {
        return dark;
      },
      media: query,
      addEventListener: (_: string, f: () => void) => {
        listener = f;
      },
      removeEventListener: () => {},
    }));

    renderIn(
      <SettingsProvider>
        <ThemeProvider>
          <span>content</span>
        </ThemeProvider>
      </SettingsProvider>,
    );
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("light"));

    dark = true;
    await act(async () => {
      listener?.();
    });
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"));
    vi.unstubAllGlobals();
  });

  it("does not fall over when the core cannot be asked", async () => {
    // The settings live in a database now, and a database can be unreachable. Appearance
    // is not worth a screen a person cannot get past — the system's theme will do.
    mockSettingsGet.mockRejectedValue({ code: "STORAGE_FAILED", details: [] });
    renderIn(
      <SettingsProvider>
        <ThemeProvider>
          <span>content</span>
        </ThemeProvider>
      </SettingsProvider>,
    );
    expect(screen.getByText("content")).toBeInTheDocument();
    await waitFor(() => {
      expect(["light", "dark"]).toContain(document.documentElement.dataset.theme);
    });
  });
});
