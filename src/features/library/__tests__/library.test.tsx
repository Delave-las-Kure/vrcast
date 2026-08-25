/**
 * T059 — тесты раздела библиотеки.
 *
 * Проверяется то, чем библиотека отличается от списка файлов: нераспознанное видно,
 * пропавшее помечено, удаление называет последствия, а недоступный сервер не
 * превращается в пустой экран.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
  const actual =
    await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
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

const draw = () =>
  render(
    <MemoryRouter>
      <LibraryScreen />
    </MemoryRouter>,
  );

beforeEach(() => {
  vi.clearAllMocks();
  useServers.setState({ profiles: [profile()], loading: false, error: null });
  mockServersList.mockResolvedValue([profile()]);
  mockLibraryList.mockResolvedValue(view());
});

describe("библиотека", () => {
  it("без активного сервера ведёт к его добавлению, а не молчит", async () => {
    useServers.setState({ profiles: [], loading: false, error: null });
    mockServersList.mockResolvedValue([]);
    draw();

    expect(await screen.findByText(/Активный сервер не выбран/)).toBeInTheDocument();
    expect(screen.getByText("Перейти к серверам")).toBeInTheDocument();
  });

  it("показывает медиа с числом файлов и объёмом", async () => {
    draw();
    expect(await screen.findByText("Название фильма")).toBeInTheDocument();
    expect(screen.getByText(/1 файл · 1,5 ГБ/)).toBeInTheDocument();
  });

  it("раскрывает медиа и показывает параметры файла", async () => {
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText("film_22.mp4")).toBeInTheDocument();
    expect(screen.getByText("3840×2160")).toBeInTheDocument();
    expect(screen.getByText("1:02:05")).toBeInTheDocument();
    expect(screen.getByText("22,0 Мбит/с")).toBeInTheDocument();
  });

  it("не придумывает параметры там, где заголовок не прочитан", async () => {
    // «—» значит «мы не смогли прочитать». Ноль читался бы как «в файле этого нет».
    mockLibraryList.mockResolvedValue(
      view({
        media: [
          media({
            files: [
              file({ width: null, height: null, duration_s: null, bitrate_bps: null }),
            ],
          }),
        ],
      }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findAllByText("—")).toHaveLength(3);
  });

  it("предупреждает о файле, который зритель не сможет начать смотреть сразу", async () => {
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ files: [file({ faststart_ok: false })] })] }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText(/Заголовок не в начале файла/)).toBeInTheDocument();
  });

  it("помечает пропавший файл и не предлагает его ссылку", async () => {
    // FR-018: файл удалили мимо приложения. Ссылка на него не работает,
    // и предлагать её копировать — обманывать.
    mockLibraryList.mockResolvedValue(
      view({ media: [media({ files: [file({ exists_on_server: false })] })] }),
    );
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));

    expect(await screen.findByText(/Файла нет на сервере/)).toBeInTheDocument();
    expect(screen.getByText("ссылка нерабочая")).toBeInTheDocument();
    expect(screen.queryByText("Копировать ссылку")).not.toBeInTheDocument();
  });

  it("предлагает обе ссылки, когда задан CDN", async () => {
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

    expect(await screen.findByText("Ссылка с сервера")).toBeInTheDocument();
    expect(screen.getByText("Ссылка через CDN")).toBeInTheDocument();
  });
});

describe("нераспознанное", () => {
  it("показывается отдельной группой, а не прячется", async () => {
    // FR-015. Файл, которого не видно в приложении, всё равно занимает место
    // и всё равно раздаётся по прямой ссылке.
    mockLibraryList.mockResolvedValue(
      view({ unrecognized: [file({ path: "одинокий ролик.mp4" })] }),
    );
    draw();

    fireEvent.click(await screen.findByText("Не распознано"));
    expect(await screen.findByText("одинокий ролик.mp4")).toBeInTheDocument();
    expect(screen.getByText(/не числятся ни за одним медиа/)).toBeInTheDocument();
  });

  it("позволяет отнести файл к медиа, но не делает этого сам", async () => {
    mockLibraryList.mockResolvedValue(
      view({ unrecognized: [file({ path: "чужой.mp4" })] }),
    );
    draw();

    fireEvent.click(await screen.findByText("Не распознано"));
    const select = await screen.findByLabelText("Отнести к медиа");
    expect(mockFileMove).not.toHaveBeenCalled();

    fireEvent.change(select, { target: { value: "m1" } });
    await waitFor(() =>
      expect(mockFileMove).toHaveBeenCalledWith("srv_1", "чужой.mp4", "m1", true),
    );
  });
});

describe("удаление", () => {
  it("спрашивает подтверждение и показывает последствия от ядра", async () => {
    // Первый вызов — без подтверждения — ядро отклоняет и называет числа.
    // Именно их и показывает диалог: интерфейс не сочиняет своих формулировок.
    const refusal: AppError = {
      code: "CONFIRMATION_REQUIRED",
      message: "Удалить «Название фильма»? Будет снято 3 файла, освободится 4,2 ГБ.",
      hint: "Прочитайте, что именно произойдёт, и подтвердите.",
    };
    mockMediaDelete.mockRejectedValueOnce(refusal);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText("Удалить медиа"));

    expect(await screen.findByText(refusal.message)).toBeInTheDocument();
    expect(mockMediaDelete).toHaveBeenCalledWith("srv_1", "m1", false);
    expect(mockMediaDelete).not.toHaveBeenCalledWith("srv_1", "m1", true);

    mockMediaDelete.mockResolvedValueOnce("m1");
    fireEvent.click(screen.getByText("Удалить"));
    await waitFor(() =>
      expect(mockMediaDelete).toHaveBeenCalledWith("srv_1", "m1", true),
    );
  });

  it("отказ от удаления ничего не удаляет", async () => {
    mockMediaDelete.mockRejectedValueOnce({
      code: "CONFIRMATION_REQUIRED",
      message: "Удалить «Название фильма»? Будет снят 1 файл.",
      hint: "Подтвердите.",
    } satisfies AppError);
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText("Удалить медиа"));
    fireEvent.click(await screen.findByText("Не удалять"));

    await waitFor(() =>
      expect(screen.queryByText(/Будет снят 1 файл/)).not.toBeInTheDocument(),
    );
    expect(mockMediaDelete).not.toHaveBeenCalledWith("srv_1", "m1", true);
  });
});

describe("переименование", () => {
  it("предупреждает о поломке ссылок только при смене короткого имени", async () => {
    draw();
    fireEvent.click(await screen.findByText("Название фильма"));
    fireEvent.click(await screen.findByText("Переименовать"));

    const слово = /все выданные раньше ссылки перестанут работать/;
    expect(screen.queryByText(слово)).not.toBeInTheDocument();

    // Меняем только название — предупреждения быть не должно.
    fireEvent.change(screen.getByLabelText("Название"), {
      target: { value: "Другое название" },
    });
    expect(screen.queryByText(слово)).not.toBeInTheDocument();

    // А вот смена короткого имени переименует файлы на сервере.
    fireEvent.change(screen.getByLabelText("Короткое имя"), {
      target: { value: "drugoe" },
    });
    expect(await screen.findByText(слово)).toBeInTheDocument();
  });
});

describe("недоступный сервер", () => {
  it("показывает последнее известное с пометкой, а не пустой экран", async () => {
    mockLibraryList.mockResolvedValue(view({ stale: true }));
    draw();

    expect(await screen.findByText("Сервер сейчас недоступен")).toBeInTheDocument();
    // Данные при этом на месте: библиотека не пропала, пропала связь.
    expect(screen.getByText("Название фильма")).toBeInTheDocument();
  });

  it("не даёт менять библиотеку, пока связи нет", async () => {
    mockLibraryList.mockResolvedValue(view({ stale: true }));
    draw();

    fireEvent.click(await screen.findByText("Название фильма"));
    expect(await screen.findByText("Удалить медиа")).toBeDisabled();
    expect(screen.getByText("Переименовать")).toBeDisabled();
  });
});

describe("место на диске", () => {
  it("показывает свободное, общее и занятое видео", async () => {
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

    const bar = screen.getByRole("progressbar", {
      name: "Занято места на диске сервера",
    });
    expect(bar).toHaveAttribute("aria-valuenow", "75");
  });
});
