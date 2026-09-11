/**
 * T059 — the library section.
 *
 * What is checked is what makes a library different from a list of files: the unrecognised
 * is visible, what has gone missing is marked, deleting names its consequences, and a
 * server out of reach does not turn into an empty screen.
 *
 * **The Cyrillic that is left is deliberate.** The media are called `Забытый фильм` and a
 * file `странный файл.mp4`, because that is what this project's own library holds; and the
 * assertions about `3 файла` and `22,0 Мбит/с` are about Russian counting and Russian
 * number formatting, which is the very thing they check.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { en, renderIn, ru } from "../../../test-utils";
import type {
  AppError,
  FileView,
  GroupSuggestion,
  LadderSetView,
  LibraryView,
  MediaView,
  ServerProfile,
} from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockLibraryList = vi.fn<() => Promise<LibraryView>>();
const mockSuggestGroups = vi.fn<() => Promise<GroupSuggestion>>();
const mockMediaDelete = vi.fn();
const mockFileDelete = vi.fn();
const mockFileMove = vi.fn();
const mockMediaCreate = vi.fn();
const mockMediaRename = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      serversList: () => mockServersList(),
      serverSetActive: vi.fn(),
      libraryList: (...a: unknown[]) => mockLibraryList(...(a as [])),
      // The stub's default is an empty **list**, and this one answers with an object — so
      // without a line here the screen reads `.groups` off an array and the whole library
      // goes white. Named rather than left to the default for that reason (T470, T480).
      librarySuggestGroups: () => mockSuggestGroups(),
      mediaCreate: (...a: unknown[]) => mockMediaCreate(...a),
      mediaRename: (...a: unknown[]) => mockMediaRename(...a),
      mediaDelete: (...a: unknown[]) => mockMediaDelete(...a),
      fileMove: (...a: unknown[]) => mockFileMove(...a),
      fileDelete: (...a: unknown[]) => mockFileDelete(...a),
      linksFor: vi.fn(),
    }),
    onLibraryChanged: vi.fn(async () => () => {}),
    // The card counts its viewers off the same stream (T176). Without this the real
    // listener runs and reaches for the shell, which is not there in a test.
    onViewersUpdate: vi.fn(async () => () => {}),
  };
});

const { LibraryScreen } = await import("../LibraryScreen");
const { useServers } = await import("../../servers/store");

function profile(): ServerProfile {
  return {
    id: "srv_1",
    name: "Мой сервер",
    host: "203.0.113.10",
    port: 22,
    user: "root",
    auth_kind: "key",
    secret_ref: "server/srv_1",
    key_path: null,
    domain: "stream.example.com",
    video_dir: "/srv/раздача/видео",
    cdn_base: null,
    host_fingerprint: "SHA256:x",
    ipv6_mode: null,
    is_active: true,
  };
}

function file(over: Partial<FileView> = {}): FileView {
  return {
    path: "film_22.mp4",
    size_bytes: 1024 * 1024 * 1500,
    duration_s: 3725,
    width: 3840,
    height: 2160,
    bitrate_bps: 22_000_000,
    video_codec: "h264",
    audio_codec: "aac",
    faststart_ok: true,
    exists_on_server: true,
    origin_url: "https://stream.example.com/videos/film_22.mp4",
    cdn_url: null,
    ...over,
  };
}

function media(over: Partial<MediaView> = {}): MediaView {
  return {
    id: "m1",
    title: "Название фильма",
    slug: "nazvanie-filma",
    files: [file()],
    ladders: [],
    total_bytes: 1024 * 1024 * 1500,
    created_at: "2026-08-01T10:00:00Z",
    ...over,
  };
}

function ladderSet(over: Partial<LadderSetView> = {}): LadderSetView {
  return {
    path: "nazvanie-filma/master.m3u8",
    size_bytes: 1024 * 1024 * 900,
    width: 1920,
    height: 1080,
    bitrate_bps: 5_000_000,
    duration_s: 3725,
    exists_on_server: true,
    origin_url: "https://stream.example.com/videos/nazvanie-filma/master.m3u8",
    cdn_url: null,
    ...over,
  };
}

function view(over: Partial<LibraryView> = {}): LibraryView {
  return {
    server_id: "srv_1",
    media: [media()],
    unrecognized: [],
    disk: null,
    stale: false,
    ...over,
  };
}

const draw = (lang: "ru" | "en" = "ru") =>
  renderIn(
    <MemoryRouter>
      <LibraryScreen />
    </MemoryRouter>,
    lang,
  );

beforeEach(() => {
  vi.clearAllMocks();
  useServers.setState({ profiles: [profile()], loading: false, error: null });
  mockServersList.mockResolvedValue([profile()]);
  mockSuggestGroups.mockResolvedValue({ groups: [], singles: [] });
  mockLibraryList.mockResolvedValue(view());
});

describe("the library", () => {
  it("says the server is out of reach rather than showing nothing", async () => {
    useServers.setState({ profiles: [], loading: false, error: null });
    mockServersList.mockResolvedValue([]);
    draw();

    expect(await screen.findByText(ru.ui.library.noActiveServer)).toBeInTheDocument();
    expect(screen.getByText(ru.ui.library.goToServers)).toBeInTheDocument();
  });

  it("shows a medium with how many files it has and how much they weigh", async () => {
    draw();
    expect(await screen.findByText("Название фильма")).toBeInTheDocument();
    expect(screen.getByText(/1 файл · 1,5 ГБ/)).toBeInTheDocument();
  });

  it("shows each file with what is actually in it", async () => {
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText("film_22.mp4")).toBeInTheDocument();
    expect(screen.getByText("3840×2160")).toBeInTheDocument();
    expect(screen.getByText("1:02:05")).toBeInTheDocument();
    expect(screen.getByText("22,0 Мбит/с")).toBeInTheDocument();
  });

  it("does not work out again what the server has not changed", async () => {
    // The same answer as before is no reason to redraw: a screen that flickers on every answer teaches a person the application is unwell.
    mockLibraryList.mockResolvedValue(
      view({
        media: [
          media({
            files: [file({ width: null, height: null, duration_s: null, bitrate_bps: null })],
          }),
        ],
      }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findAllByText("—")).toHaveLength(3);
  });

  it("goes back to the medium a file was moved out of", async () => {
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ files: [file({ faststart_ok: false })] })] }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText(ru.ui.library.faststartWarning)).toBeInTheDocument();
  });

  it("names what a deletion would cost and does not do it unasked", async () => {
    // FR-018: the files go with the medium. Nobody is to find that out afterwards, and
    // certainly not from the space freed up on the disk.
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ files: [file({ exists_on_server: false })] })] }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText(ru.ui.library.missingWarning)).toBeInTheDocument();
    expect(screen.getByText(ru.ui.library.linkDead)).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.library.linkCopy)).not.toBeInTheDocument();
  });

  it("shows both links when a CDN is set", async () => {
    mockLibraryList.mockResolvedValue(
      view({
        media: [
          media({
            files: [file({ cdn_url: "https://cdn.example.net/videos/film_22.mp4" })],
          }),
        ],
      }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText(ru.ui.library.linkFromServer)).toBeInTheDocument();
    expect(screen.getByText(ru.ui.library.linkViaCdn)).toBeInTheDocument();
  });

  it("renders a several-thousand-character title whole, without truncating it (T566)", async () => {
    // The backend puts no ceiling on a title's length (MAX_SLUG_LEN exists for `slug`
    // because it becomes a file name; a title never does, so there is no filesystem-shaped
    // limit to enforce — see the backend test for T566 in tests/integration/library_ops.rs).
    // What the screen owes a person who typed one anyway is that the title still shows up
    // whole: not silently cut short, which would look like their own text was lost.
    //
    // One long word with no spaces, deliberately: a title that wraps at ordinary word
    // boundaries would render fine with no CSS help at all, and would tell this test
    // nothing about `.media__title`'s own overflow handling.
    const longTitle = "Оченьдлинноеназваниефильмабезединогопробела".repeat(100);
    mockLibraryList.mockResolvedValue(view({ media: [media({ title: longTitle })] }));
    draw();

    const title = await screen.findByText(longTitle);
    expect(title).toBeInTheDocument();
    expect(title).toHaveClass("media__title");
    expect(title.textContent).toBe(longTitle);
  });
});

describe("a medium's built quality sets (T529)", () => {
  it("shows every parameter known about a set, and offers to copy its link", async () => {
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ ladders: [ladderSet()] })] }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    const list = await screen.findByTestId("ladder-sets-m1");
    expect(list).toHaveTextContent("nazvanie-filma/master.m3u8");
    expect(list).toHaveTextContent("1920×1080");
    expect(list).toHaveTextContent("1:02:05");
    expect(list).toHaveTextContent("5,0 Мбит/с");
    expect(list.querySelector(`.copy-link`)).toBeTruthy();
    expect(within(list).getByText(ru.ui.library.linkCopy)).toBeInTheDocument();
  });

  it("does not show a placeholder where a set's parameters are unknown", async () => {
    // A set not built by this application, or one whose `.facts` could not be read: honest
    // absence, not a made-up zero or dash that would look like a measurement.
    mockLibraryList.mockResolvedValue(
      view({
        media: [
          media({
            ladders: [
              ladderSet({ width: null, height: null, bitrate_bps: null, duration_s: null }),
            ],
          }),
        ],
      }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    const list = await screen.findByTestId("ladder-sets-m1");
    expect(list).not.toHaveTextContent("×");
    expect(list).not.toHaveTextContent("Мбит/с");
    // The link is still offered: an unmeasured set is still a real, servable file.
    expect(within(list).getByText(ru.ui.library.linkCopy)).toBeInTheDocument();
  });

  it("marks a set missing on the server the same way a file is marked", async () => {
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ ladders: [ladderSet({ exists_on_server: false })] })] }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    const list = await screen.findByTestId("ladder-sets-m1");
    expect(within(list).getByText(ru.ui.library.linkDead)).toBeInTheDocument();
  });
});

describe("what was not recognised", () => {
  it("shows what it found rather than hiding it", async () => {
    // FR-015. A file that reached the server and was not matched to a medium is not the
    // person's mistake, and it must not vanish from view.
    mockLibraryList.mockResolvedValue(
      view({ unrecognized: [file({ path: "одинокий ролик.mp4" })] }),
    );
    draw();

    fireEvent.click(await screen.findByText(ru.ui.library.unrecognizedTitle));
    expect(await screen.findByText("одинокий ролик.mp4")).toBeInTheDocument();
    expect(screen.getByText(ru.ui.library.unrecognizedNote)).toBeInTheDocument();
  });

  it("says what it thinks belongs together, and groups nothing by itself", async () => {
    // T480. The core has been able to work this out since milestone A and nothing called it:
    // the module was reachable from no command, no task and no screen until the reachability
    // guard found it. What is checked here is that the suggestion reaches the screen, and
    // that it stays a suggestion — the files are still assigned one at a time, by hand.
    mockLibraryList.mockResolvedValue(
      view({
        unrecognized: [file({ path: "Backrooms_10.mp4" }), file({ path: "Backrooms_22.mp4" })],
      }),
    );
    mockSuggestGroups.mockResolvedValue({
      groups: [
        {
          key: "backrooms",
          suggested_title: "Backrooms",
          files: ["Backrooms_10.mp4", "Backrooms_22.mp4"],
          reason: "BITRATE_VARIANTS",
        },
      ],
      singles: [],
    });
    draw();

    fireEvent.click(await screen.findByText(ru.ui.library.unrecognizedTitle));
    expect(await screen.findByText("Backrooms")).toBeInTheDocument();
    expect(screen.getByTestId("group-suggestion")).toHaveTextContent(
      ru.ui.library.groupReason.BITRATE_VARIANTS,
    );
    // And nothing was moved: a suggestion that acted on itself would be the very thing this
    // screen exists to avoid.
    expect(mockFileMove).not.toHaveBeenCalled();
  });

  it("lets a file be moved from one medium to another", async () => {
    // ⚠ **T530, FR-013.** `file_move` has always existed and was reachable from one place:
    // assigning a file the catalogue had never heard of. From one medium to another there was
    // no way at all — which is the half a person needs after choosing the wrong medium at
    // upload, and by T505 that used to happen every single time.
    mockLibraryList.mockResolvedValue(
      view({
        media: [
          media({ id: "m1", title: "The first", files: [file({ path: "film.mp4" })] }),
          media({ id: "m2", title: "The second", slug: "second", files: [] }),
        ],
      }),
    );
    draw();

    // The files of a medium open inside it, so the card is opened first — the same step a
    // person takes.
    fireEvent.click(await screen.findByText("The first"));
    const select = await screen.findByLabelText(ru.ui.library.moveTo);
    expect(mockFileMove).not.toHaveBeenCalled();

    fireEvent.change(select, { target: { value: "m2" } });
    await waitFor(() =>
      expect(mockFileMove).toHaveBeenCalledWith("srv_1", "film.mp4", "m2", true),
    );
  });

  it("does not offer to move a file to the medium it is already in", async () => {
    // Work that changes nothing, offered as though it were a choice. With one medium there is
    // nowhere to move to and the control is not there at all.
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ files: [file({ path: "film.mp4" })] })] }),
    );
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    // The file is there, so the card really is open and the absence below means something.
    expect(await screen.findByText("film.mp4")).toBeInTheDocument();
    expect(screen.queryByLabelText(ru.ui.library.moveTo)).toBeNull();
  });

  it("lets a file be tied to a medium without touching its name", async () => {
    mockLibraryList.mockResolvedValue(view({ unrecognized: [file({ path: "чужой.mp4" })] }));
    draw();

    fireEvent.click(await screen.findByText(ru.ui.library.unrecognizedTitle));
    const select = await screen.findByLabelText(ru.ui.library.assignTo);
    expect(mockFileMove).not.toHaveBeenCalled();

    fireEvent.change(select, { target: { value: "m1" } });
    await waitFor(() =>
      expect(mockFileMove).toHaveBeenCalledWith("srv_1", "чужой.mp4", "m1", true),
    );
  });
});

describe("deleting", () => {
  it("names the consequences and asks before doing anything", async () => {
    // Deleting a medium takes its files with it, and that is the whole of the question.
    // Asked afterwards it would be a report, and a report about a deletion is of no use.
    const refusal: AppError = {
      code: "CONFIRMATION_REQUIRED",
      details: [
        {
          key: "CONFIRM_DELETE",
          params: { what: "Название фильма", files: 3, bytes: 4_509_715_660 },
        },
      ],
    };
    mockMediaDelete.mockRejectedValueOnce(refusal);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.deleteMedia));

    // The numbers come from the core; the sentence around them is ours, and it
    // counts in Russian: three files is «3 файла», not «3 файл».
    const spelled = await screen.findByText(/Будет снято 3 файла/);
    expect(spelled).toBeInTheDocument();
    expect(spelled.textContent).toContain("4,2 ГБ");
    expect(mockMediaDelete).toHaveBeenCalledWith("srv_1", "m1", false);
    expect(mockMediaDelete).not.toHaveBeenCalledWith("srv_1", "m1", true);

    mockMediaDelete.mockResolvedValueOnce("m1");
    fireEvent.click(screen.getByText(ru.ui.library.deleteYes));
    await waitFor(() => expect(mockMediaDelete).toHaveBeenCalledWith("srv_1", "m1", true));
  });

  it("declining a deletion deletes nothing", async () => {
    mockMediaDelete.mockRejectedValueOnce({
      code: "CONFIRMATION_REQUIRED",
      details: [
        {
          key: "CONFIRM_DELETE",
          params: { what: "Название фильма", files: 1, bytes: 1024 },
        },
      ],
    } satisfies AppError);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.deleteMedia));
    fireEvent.click(await screen.findByText(ru.ui.library.deleteNo));

    await waitFor(() => expect(screen.queryByText(/Будет снято 1 файл/)).not.toBeInTheDocument());
    expect(mockMediaDelete).not.toHaveBeenCalledWith("srv_1", "m1", true);
  });

  it("T581 — a confirmed delete that fails shows the failure in the dialog instead of hanging", async () => {
    // The first, unconfirmed call is refused with CONFIRMATION_REQUIRED as always — that
    // is not the bug. The bug is what happens to the *second*, confirmed call: until T581
    // its failure had nowhere to be drawn, so the dialog just sat there, busy forever.
    const refusal: AppError = {
      code: "CONFIRMATION_REQUIRED",
      details: [
        {
          key: "CONFIRM_DELETE",
          params: { what: "Название фильма", files: 3, bytes: 4_509_715_660 },
        },
      ],
    };
    mockMediaDelete.mockRejectedValueOnce(refusal);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.deleteMedia));
    await screen.findByText(/Будет снято 3 файла/);

    mockMediaDelete.mockRejectedValueOnce({ code: "INTERNAL" } satisfies AppError);
    fireEvent.click(screen.getByText(ru.ui.library.deleteYes));

    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() =>
      expect(within(dialog).getByText(ru.errors.INTERNAL.message)).toBeInTheDocument(),
    );

    // The dialog stayed open — both buttons are still reachable — and busy was
    // lifted, so trying again is actually possible rather than a dead button.
    expect(within(dialog).getByText(ru.ui.library.deleteYes)).not.toBeDisabled();
    expect(within(dialog).getByText(ru.ui.library.deleteNo)).not.toBeDisabled();
  });

  it("T581 — a confirmed file delete that fails also shows the failure in the dialog", async () => {
    // Same wrapper (`act`), same bug class, different confirmed call (fileDelete
    // instead of mediaDelete) — worth its own test since nothing else covered this path.
    const refusal: AppError = {
      code: "CONFIRMATION_REQUIRED",
      details: [{ key: "CONFIRM_DELETE", params: { what: "film_22.mp4", files: 1, bytes: 1024 } }],
    };
    mockFileDelete.mockRejectedValueOnce(refusal);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.deleteFile));
    await screen.findByRole("alertdialog");

    mockFileDelete.mockRejectedValueOnce({ code: "INTERNAL" } satisfies AppError);
    fireEvent.click(screen.getByText(ru.ui.library.deleteYes));

    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() =>
      expect(within(dialog).getByText(ru.errors.INTERNAL.message)).toBeInTheDocument(),
    );
  });

  it("T584 — a rename error cancelled away does not leak into the next dialog opened", async () => {
    // The rename dialog fails once with INTERNAL, the person cancels out of it
    // (not retries, not closes-on-success — just Cancel), and then opens a
    // *different* dialog (delete) without ever having submitted anything of its
    // own. That dialog must start clean, not inherit the abandoned rename's error.
    mockMediaRename.mockRejectedValueOnce({ code: "INTERNAL" } satisfies AppError);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.renameMedia));

    const renameDialog = (
      await screen.findByText(ru.ui.library.fieldSlug)
    ).closest("form") as HTMLElement;
    fireEvent.change(screen.getByLabelText(ru.ui.library.fieldSlug), {
      target: { value: "drugoe" },
    });
    fireEvent.click(within(renameDialog).getByRole("button", { name: ru.ui.library.rename }));

    await waitFor(() =>
      expect(within(renameDialog).getByText(ru.errors.INTERNAL.message)).toBeInTheDocument(),
    );

    // Cancel out of the rename dialog — this is the path T584 covers: onCancel,
    // not a successful/act()-wrapped submit.
    fireEvent.click(within(renameDialog).getByText(ru.ui.common.cancel));
    await waitFor(() => expect(screen.queryByLabelText(ru.ui.library.fieldSlug)).toBeNull());

    mockMediaDelete.mockRejectedValueOnce({
      code: "CONFIRMATION_REQUIRED",
      details: [
        { key: "CONFIRM_DELETE", params: { what: "Название фильма", files: 1, bytes: 1024 } },
      ],
    } satisfies AppError);
    fireEvent.click(screen.getByText(ru.ui.library.deleteMedia));

    await screen.findByRole("alertdialog");
    expect(screen.queryByText(ru.errors.INTERNAL.message)).not.toBeInTheDocument();
  });
});

describe("renaming", () => {
  it("asks for the new name and hands it to the core", async () => {
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.renameMedia));

    const warning = ru.ui.library.slugChangeWarning;
    expect(screen.queryByText(warning)).not.toBeInTheDocument();

    // Changing only the title — there must be no warning.
    fireEvent.change(screen.getByLabelText(ru.ui.library.fieldTitle), {
      target: { value: "Другое название" },
    });
    expect(screen.queryByText(warning)).not.toBeInTheDocument();

    // Changing the short name, on the other hand, renames the files on the server.
    fireEvent.change(screen.getByLabelText(ru.ui.library.fieldSlug), {
      target: { value: "drugoe" },
    });
    expect(await screen.findByText(warning)).toBeInTheDocument();
  });

  it("submits the first attempt as not yet confirmed and closes on success", async () => {
    // T545. Nobody has confirmed anything yet — an ordinary rename with a free slug
    // goes straight through, and the dialog closes exactly as it always has.
    mockMediaRename.mockResolvedValueOnce(undefined);
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.renameMedia));

    const dialog = (await screen.findByText(ru.ui.library.fieldSlug)).closest("form") as HTMLElement;

    fireEvent.change(screen.getByLabelText(ru.ui.library.fieldSlug), {
      target: { value: "drugoe" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: ru.ui.library.rename }));

    await waitFor(() =>
      expect(mockMediaRename).toHaveBeenCalledWith("srv_1", "m1", null, "drugoe", false),
    );
    await waitFor(() => expect(screen.queryByLabelText(ru.ui.library.fieldSlug)).toBeNull());
  });

  it("offers to rename anyway when the file is being watched, without losing what was typed", async () => {
    // T545: media_rename refuses with FILE_IN_USE (a fixed warning, not one the core
    // composes with numbers) when the slug changes on a medium with active viewers.
    // The dialog must show a way to go on, and the title/slug typed so far must
    // survive the round-trip rather than reopening as a fresh, empty form.
    const refusal: AppError = { code: "FILE_IN_USE" };
    mockMediaRename.mockRejectedValueOnce(refusal);
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText(ru.ui.library.renameMedia));

    const dialog = (await screen.findByText(ru.ui.library.fieldSlug)).closest("form") as HTMLElement;

    fireEvent.change(screen.getByLabelText(ru.ui.library.fieldTitle), {
      target: { value: "Другое название" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.library.fieldSlug), {
      target: { value: "drugoe" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: ru.ui.library.rename }));

    await waitFor(() =>
      expect(mockMediaRename).toHaveBeenCalledWith(
        "srv_1",
        "m1",
        "Другое название",
        "drugoe",
        false,
      ),
    );

    // The warning is visible and the form is still the rename form, values intact.
    const anyway = await within(dialog).findByText(ru.ui.library.renameAnyway);
    expect(screen.getByDisplayValue("Другое название")).toBeInTheDocument();
    expect(screen.getByDisplayValue("drugoe")).toBeInTheDocument();

    mockMediaRename.mockResolvedValueOnce(undefined);
    fireEvent.click(anyway);

    await waitFor(() =>
      expect(mockMediaRename).toHaveBeenCalledWith(
        "srv_1",
        "m1",
        "Другое название",
        "drugoe",
        true,
      ),
    );
  });
});

describe("a server out of reach", () => {
  it("shows the last that was known, marked as such, rather than an empty screen", async () => {
    mockLibraryList.mockResolvedValue(view({ stale: true }));
    draw();

    expect(await screen.findByText(ru.ui.library.staleTitle)).toBeInTheDocument();
    // The point is not the mark itself: the library stays usable, only stale.
    expect(screen.getByText("Название фильма")).toBeInTheDocument();
  });

  it("lets nothing be changed while the server is out of reach", async () => {
    mockLibraryList.mockResolvedValue(view({ stale: true }));
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    expect(await screen.findByText(ru.ui.library.deleteMedia)).toBeDisabled();
    expect(screen.getByText(ru.ui.library.renameMedia)).toBeDisabled();
  });
});

describe("room on the disk", () => {
  it("shows what is free when the server said so", async () => {
    mockLibraryList.mockResolvedValue(
      view({
        disk: {
          total_bytes: 1024 ** 3 * 100,
          free_bytes: 1024 ** 3 * 25,
          used_by_videos_bytes: 1024 ** 3 * 60,
        },
      }),
    );
    draw();

    expect(await screen.findByText(/25,0 ГБ/)).toBeInTheDocument();
    expect(screen.getByText(/видео занимают 60,0 ГБ/)).toBeInTheDocument();

    const bar = screen.getByRole("progressbar", { name: ru.ui.library.diskLabel });
    expect(bar).toHaveAttribute("aria-valuenow", "75");
  });

  it("writes the same figures in English units when English is chosen", async () => {
    // The arithmetic is shared, so the two languages can never disagree about how
    // full the disk is — only about how the number is written.
    mockLibraryList.mockResolvedValue(
      view({
        disk: {
          total_bytes: 1024 ** 3 * 100,
          free_bytes: 1024 ** 3 * 25,
          used_by_videos_bytes: 1024 ** 3 * 60,
        },
      }),
    );
    draw("en");

    expect(await screen.findByText(/25\.0 GB/)).toBeInTheDocument();
    const bar = screen.getByRole("progressbar", { name: en.ui.library.diskLabel });
    expect(bar).toHaveAttribute("aria-valuenow", "75");
  });
});
