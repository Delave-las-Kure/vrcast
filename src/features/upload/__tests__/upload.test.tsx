/**
 * T103 — the upload screen and the queue.
 *
 * The core is stood in for: a check of the interface must need neither a server nor a
 * database. What is checked is what a person sees — and above all the difference between
 * a question and a refusal. Showing "not enough room" as a warning with an "upload
 * anyway" button would be a lie: agreeing does not make room appear.
 *
 * **The Cyrillic that is left is deliberate.** The file is called `фильм 22.mp4` and the
 * serving directory is `/srv/проба/видео`, because that is what this project's own files
 * are called, and a check that only ever sees Latin names proves nothing about them.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { en, renderIn, ru } from "../../../test-utils";
import { fill } from "../../../shared/i18n/render";
import type { AppError, LibraryView, ServerProfile, Task, TaskOnClose } from "../../../shared/contract";

const mockUploadStart = vi.fn<(request: unknown) => Promise<string>>();
const mockLibraryList = vi.fn<() => Promise<LibraryView>>();
const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockOpen = vi.fn<() => Promise<string | null>>();
const mockTasksReorder = vi.fn<(ids: string[]) => Promise<number>>();

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => mockOpen() }));

vi.mock("../../../shared/ipc", async () => {
  const actual =
    await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      serversList: () => mockServersList(),
      serverSetActive: vi.fn(),
      libraryList: () => mockLibraryList(),
      uploadStart: (request: unknown) => mockUploadStart(request),
      uploadResume: vi.fn(),
      tasksReorder: (ids: string[]) => mockTasksReorder(ids),
    },
  };
});

const { UploadScreen } = await import("../UploadScreen");
const { QueueOrder } = await import("../../tasks/QueueOrder");
const { CloseConsequences } = await import("../../tasks/CloseConsequences");

function profile(over: Partial<ServerProfile> = {}): ServerProfile {
  return {
    id: "s1",
    name: "Боевой",
    host: "203.0.113.10",
    port: 22,
    user: "root",
    auth_kind: "password",
    key_path: null,
    domain: "stream.example.com",
    // Deliberately not the default path: a stand-in that repeats it will one day pass a
    // check in a place where the real value has nowhere to come from (FR-004).
    video_dir: "/srv/проба/видео",
    cdn_base: null,
    ipv6_mode: null,
    is_active: true,
    host_fingerprint: "SHA256:aaa",
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    ...over,
  } as ServerProfile;
}

const EMPTY_LIBRARY: LibraryView = {
  server_id: "s1",
  media: [],
  unrecognized: [],
  disk: null,
  stale: false,
};

beforeEach(() => {
  vi.clearAllMocks();
  mockServersList.mockResolvedValue([profile()]);
  mockLibraryList.mockResolvedValue(EMPTY_LIBRARY);
  mockOpen.mockResolvedValue("F:\\видео\\фильм 22.mp4");
  mockUploadStart.mockResolvedValue("t-1");
  mockTasksReorder.mockResolvedValue(2);
});

/** Choose a file and wait for the screen to notice. */
async function chooseAFile() {
  fireEvent.click(await screen.findByText(ru.ui.upload.pickFile));
  await screen.findByDisplayValue("фильм 22.mp4");
}

describe("the upload screen", () => {
  it("takes the served name from the name of the file chosen", async () => {
    // It is what is wanted most of the time. Making a person retype it is extra work and
    // one more chance to mistype.
    renderIn(<UploadScreen />);
    await chooseAFile();
    expect(screen.getByLabelText(ru.ui.upload.fieldName)).toHaveValue("фильм 22.mp4");
  });

  it("will not upload until a file has been chosen", async () => {
    renderIn(<UploadScreen />);
    expect(await screen.findByText(ru.ui.upload.start)).toBeDisabled();
  });

  it("hands the core the path, the name and the speed limit", async () => {
    renderIn(<UploadScreen />);
    await chooseAFile();

    fireEvent.change(screen.getByLabelText(ru.ui.upload.fieldLimit), {
      target: { value: "1250000" },
    });
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    await waitFor(() => expect(mockUploadStart).toHaveBeenCalled());
    expect(mockUploadStart).toHaveBeenCalledWith(
      expect.objectContaining({
        server_id: "s1",
        local_path: "F:\\видео\\фильм 22.mp4",
        remote_name: "фильм 22.mp4",
        limit_bps: 1_250_000,
        confirmed: false,
      }),
    );
  });

  it("says the upload carries on after the application is closed", async () => {
    // FR-086. A person is not obliged to know that "in the background" here means
    // "outlives the closing": it has to be said plainly.
    renderIn(<UploadScreen />);
    await chooseAFile();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    expect(await screen.findByText(ru.ui.upload.startedHint)).toBeInTheDocument();
  });

  it("does not reach a server whose fingerprint was never confirmed", async () => {
    mockServersList.mockResolvedValue([profile({ host_fingerprint: null })]);
    renderIn(<UploadScreen />);
    expect(
      await screen.findByText(fill(ru.ui.upload.notReady, { name: "Боевой" }, ru, "ru")),
    ).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.upload.start)).not.toBeInTheDocument();
  });
});

describe("what is said before it starts", () => {
  const nameTaken: AppError = {
    code: "NAME_EXISTS",
    details: [{ key: "NAME_WILL_BE_REPLACED", params: { name: "фильм 22.mp4" } }],
  };
  const nameTakenInWords = fill(
    ru.details.NAME_WILL_BE_REPLACED,
    { name: "фильм 22.mp4" },
    ru,
    "ru",
  );

  const noRoom: AppError = {
    code: "REMOTE_DISK_FULL",
    details: [
      {
        key: "NOT_ENOUGH_SPACE",
        params: {
          short_by: 1024 ** 3 * 22,
          needed: 1024 ** 3 * 32,
          free: 1024 ** 3 * 10,
        },
      },
    ],
  };

  it("a name already taken is a question, and agreeing settles it", async () => {
    mockUploadStart.mockRejectedValueOnce(nameTaken);
    renderIn(<UploadScreen />);
    await chooseAFile();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    expect(await screen.findByText(nameTakenInWords)).toBeInTheDocument();

    // The agreement goes to the core as the same request, now confirmed.
    mockUploadStart.mockResolvedValueOnce("t-2");
    fireEvent.click(screen.getByText(ru.ui.preflight.uploadAnyway));

    await waitFor(() => expect(mockUploadStart).toHaveBeenCalledTimes(2));
    expect(mockUploadStart).toHaveBeenLastCalledWith(
      expect.objectContaining({ confirmed: true }),
    );
  });

  it("not enough room is not something agreeing can settle", async () => {
    // This difference is the whole reason the component exists. An "upload anyway" button
    // here would be a deceit: the transfer runs into the end of the disk halfway.
    mockUploadStart.mockRejectedValue(noRoom);
    renderIn(<UploadScreen />);
    await chooseAFile();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    // The numbers are the core's; the units and the separator are the language's. Asked
    // of the catalogue rather than written out here: a sentence copied into a test breaks
    // the day somebody improves the wording, and says nothing about what went wrong.
    const shortBy = fill(
      ru.details.NOT_ENOUGH_SPACE,
      noRoom.details![0].params as Record<string, number>,
      ru,
      "ru",
    );
    expect(shortBy).toContain("22,0");
    expect(await screen.findByText(shortBy)).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.preflight.uploadAnyway)).not.toBeInTheDocument();
  });

  it("the warning comes before the transfer starts, not after", async () => {
    // No task should have been made: learning about a taken name after an hour of
    // transferring is the same as not being warned at all.
    mockUploadStart.mockRejectedValue(nameTaken);
    renderIn(<UploadScreen />);
    await chooseAFile();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    await screen.findByText(nameTakenInWords);
    expect(screen.queryByText(ru.ui.upload.started)).not.toBeInTheDocument();
  });

  it("says there is not enough room in English when English is chosen", async () => {
    mockUploadStart.mockRejectedValue(noRoom);
    renderIn(<UploadScreen />, "en");
    fireEvent.click(await screen.findByText(en.ui.upload.pickFile));
    await screen.findByDisplayValue("фильм 22.mp4");
    fireEvent.click(screen.getByText(en.ui.upload.start));

    expect(await screen.findByText(/The server is 22\.0 GB short/)).toBeInTheDocument();
  });
});

describe("the queue", () => {
  function task(id: string, order: number): Task {
    return {
      id,
      kind: "upload",
      server_id: "s1",
      state: "queued",
      progress: 0,
      stage: null,
      speed_bps: null,
      eta_s: null,
      resume_token: null,
      error: null,
      queue_order: order,
      created_at: "2026-08-25T10:00:00Z",
      updated_at: "2026-08-25T10:00:00Z",
    };
  }

  it("moves a task up and hands the core the whole new order", () => {
    const onReorder = vi.fn();
    renderIn(
      <QueueOrder
        queued={[task("a", 1), task("b", 2), task("c", 3)]}
        busy={false}
        onReorder={onReorder}
      />,
    );

    // Move the third one up.
    fireEvent.click(screen.getAllByLabelText(ru.ui.tasks.moveUp)[2]);
    expect(onReorder).toHaveBeenCalledWith(["a", "c", "b"]);
  });

  it("the first has nowhere to go up and the last nowhere to go down", () => {
    renderIn(
      <QueueOrder
        queued={[task("a", 1), task("b", 2)]}
        busy={false}
        onReorder={vi.fn()}
      />,
    );
    expect(screen.getAllByLabelText(ru.ui.tasks.moveUp)[0]).toBeDisabled();
    expect(screen.getAllByLabelText(ru.ui.tasks.moveDown)[1]).toBeDisabled();
  });

  it("an empty queue is not shown at all", () => {
    const { container } = renderIn(
      <QueueOrder queued={[]} busy={false} onReorder={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe("what closing would cost", () => {
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

  it("warns when work would be lost", () => {
    renderIn(<CloseConsequences items={[carriesOn, fromTheStart]} />);
    expect(screen.getByText(ru.ui.tasks.closeLosing)).toBeInTheDocument();
    expect(
      screen.getByText(
        fill(ru.details.ON_CLOSE_RESTARTS_LOSING, { percent: 40 }, ru, "ru"),
      ),
    ).toBeInTheDocument();
  });

  it("says so when everything would carry on", () => {
    renderIn(<CloseConsequences items={[carriesOn]} />);
    expect(screen.getByText(ru.ui.tasks.closeSafe)).toBeInTheDocument();
  });

  it("stays quiet when closing is safe and there is nothing to say", () => {
    const { container } = renderIn(<CloseConsequences items={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
