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

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { en, renderIn, ru } from "../../../test-utils";
import type {
  AppError,
  FileView,
  LibraryView,
  MediaView,
  ServerProfile,
} from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockLibraryList = vi.fn<() => Promise<LibraryView>>();
const mockMediaDelete = vi.fn();
const mockFileDelete = vi.fn();
const mockFileMove = vi.fn();
const mockMediaCreate = vi.fn();
const mockMediaRename = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      serversList: () => mockServersList(),
      serverSetActive: vi.fn(),
      libraryList: (...a: unknown[]) => mockLibraryList(...(a as [])),
      mediaCreate: (...a: unknown[]) => mockMediaCreate(...a),
      mediaRename: (...a: unknown[]) => mockMediaRename(...a),
      mediaDelete: (...a: unknown[]) => mockMediaDelete(...a),
      fileMove: (...a: unknown[]) => mockFileMove(...a),
      fileDelete: (...a: unknown[]) => mockFileDelete(...a),
      linksFor: vi.fn(),
    },
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
