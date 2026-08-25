/**
 * T103 — тесты интерфейса заливки и очереди.
 *
 * Ядро подменено: тест интерфейса не должен требовать ни сервера, ни базы. Проверяется
 * то, что видит человек, — а главное, разница между вопросом и отказом. Показать
 * нехватку места как предупреждение с кнопкой «всё равно залить» значило бы соврать:
 * места от согласия не появится.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

function профиль(over: Partial<ServerProfile> = {}): ServerProfile {
  return {
    id: "s1",
    name: "Боевой",
    host: "203.0.113.10",
    port: 22,
    user: "root",
    auth_kind: "password",
    key_path: null,
    domain: "stream.example.com",
    // Нарочно не путь по умолчанию: подмена, повторяющая его, однажды пройдёт
    // проверку там, где настоящее значение брать неоткуда (FR-004).
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

const ПУСТАЯ_БИБЛИОТЕКА: LibraryView = {
  server_id: "s1",
  media: [],
  unrecognized: [],
  disk: null,
  stale: false,
};

beforeEach(() => {
  vi.clearAllMocks();
  mockServersList.mockResolvedValue([профиль()]);
  mockLibraryList.mockResolvedValue(ПУСТАЯ_БИБЛИОТЕКА);
  mockOpen.mockResolvedValue("F:\\видео\\фильм 22.mp4");
  mockUploadStart.mockResolvedValue("t-1");
  mockTasksReorder.mockResolvedValue(2);
});

/** Выбрать файл и дождаться, пока экран это заметит. */
async function выбрать_файл() {
  fireEvent.click(await screen.findByText("Выбрать файл…"));
  await screen.findByDisplayValue("фильм 22.mp4");
}

describe("экран заливки", () => {
  it("подставляет имя в раздаче из имени выбранного файла", async () => {
    // Чаще всего нужно именно оно. Заставлять человека перепечатывать имя руками —
    // лишняя работа и лишний повод для опечатки.
    render(<UploadScreen />);
    await выбрать_файл();
    expect(screen.getByLabelText("Имя в раздаче")).toHaveValue("фильм 22.mp4");
  });

  it("не даёт залить, пока файл не выбран", async () => {
    render(<UploadScreen />);
    const кнопка = await screen.findByText("Залить");
    expect(кнопка).toBeDisabled();
  });

  it("передаёт ядру путь, имя и предел скорости", async () => {
    render(<UploadScreen />);
    await выбрать_файл();

    fireEvent.change(screen.getByLabelText("Ограничить скорость"), {
      target: { value: "1250000" },
    });
    fireEvent.click(screen.getByText("Залить"));

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

  it("говорит, что заливка продолжится после закрытия приложения", async () => {
    // FR-086. Человек не обязан знать, что «в фоне» здесь значит «переживёт
    // закрытие»: об этом надо сказать прямо.
    render(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText("Залить"));

    expect(await screen.findByText(/продолжится с достигнутого места/)).toBeInTheDocument();
  });

  it("не подключается к серверу без подтверждённого отпечатка", async () => {
    mockServersList.mockResolvedValue([профиль({ host_fingerprint: null })]);
    render(<UploadScreen />);
    expect(await screen.findByText(/не подтверждён отпечаток/)).toBeInTheDocument();
    expect(screen.queryByText("Залить")).not.toBeInTheDocument();
  });
});

describe("предупреждения до старта", () => {
  const занятое_имя: AppError = {
    code: "NAME_EXISTS",
    message: "Файл «фильм 22.mp4» уже раздаётся — он будет заменён.",
    hint: "Выберите другое имя, если заменять не нужно.",
  };

  const нет_места: AppError = {
    code: "REMOTE_DISK_FULL",
    message: "На сервере не хватает 22.0 ГБ — нужно 32.0 ГБ, свободно 10.0 ГБ.",
    hint: "Освободите место на сервере или залейте файл поменьше.",
  };

  it("занятое имя показывается вопросом и снимается согласием", async () => {
    mockUploadStart.mockRejectedValueOnce(занятое_имя);
    render(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText("Залить"));

    expect(await screen.findByText(занятое_имя.message)).toBeInTheDocument();

    // Согласие уходит в ядро тем же запросом, но уже подтверждённым.
    mockUploadStart.mockResolvedValueOnce("t-2");
    fireEvent.click(screen.getByText("Всё равно залить"));

    await waitFor(() => expect(mockUploadStart).toHaveBeenCalledTimes(2));
    expect(mockUploadStart).toHaveBeenLastCalledWith(
      expect.objectContaining({ confirmed: true }),
    );
  });

  it("нехватка места согласием не снимается", async () => {
    // Ради этого различия компонент и существует. Кнопка «всё равно залить» здесь
    // была бы обманом: передача упрётся в конец диска на середине.
    mockUploadStart.mockRejectedValue(нет_места);
    render(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText("Залить"));

    expect(await screen.findByText(нет_места.message)).toBeInTheDocument();
    expect(screen.queryByText("Всё равно залить")).not.toBeInTheDocument();
  });

  it("предупреждение показывается до начала передачи, а не после", async () => {
    // Задача не должна была поставиться: узнать о занятом имени после часа
    // передачи — то же самое, что не предупреждать вовсе.
    mockUploadStart.mockRejectedValue(занятое_имя);
    render(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText("Залить"));

    await screen.findByText(занятое_имя.message);
    expect(screen.queryByText(/Заливка началась/)).not.toBeInTheDocument();
  });
});

describe("очередь", () => {
  function задача(id: string, order: number): Task {
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

  it("поднимает задачу и отдаёт ядру новый порядок целиком", () => {
    const onReorder = vi.fn();
    render(
      <QueueOrder
        queued={[задача("a", 1), задача("b", 2), задача("c", 3)]}
        busy={false}
        onReorder={onReorder}
      />,
    );

    // Поднимаем третью.
    fireEvent.click(screen.getAllByLabelText("Поднять в очереди")[2]);
    expect(onReorder).toHaveBeenCalledWith(["a", "c", "b"]);
  });

  it("первую поднять некуда, последнюю опустить некуда", () => {
    render(
      <QueueOrder queued={[задача("a", 1), задача("b", 2)]} busy={false} onReorder={vi.fn()} />,
    );
    expect(screen.getAllByLabelText("Поднять в очереди")[0]).toBeDisabled();
    expect(screen.getAllByLabelText("Опустить в очереди")[1]).toBeDisabled();
  });

  it("пустая очередь не показывается вовсе", () => {
    const { container } = render(
      <QueueOrder queued={[]} busy={false} onReorder={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe("последствия закрытия", () => {
  const продолжится: TaskOnClose = {
    id: "a",
    kind: "upload",
    progress: 0.6,
    outcome: "resumes",
    explanation: "продолжится с 60 % при следующем запуске",
  };
  const заново: TaskOnClose = {
    id: "b",
    kind: "convert",
    progress: 0.4,
    outcome: "restarts",
    explanation: "придётся начать заново — потеряется 40 % работы",
  };

  it("предупреждает, когда работа потеряется", () => {
    render(<CloseConsequences items={[продолжится, заново]} />);
    expect(screen.getByText(/часть работы потеряется/)).toBeInTheDocument();
    expect(screen.getByText(заново.explanation)).toBeInTheDocument();
  });

  it("успокаивает, когда всё продолжится", () => {
    render(<CloseConsequences items={[продолжится]} />);
    expect(screen.getByText(/можно закрыть/)).toBeInTheDocument();
  });

  it("молчит, когда закрывать безопасно и говорить не о чем", () => {
    const { container } = render(<CloseConsequences items={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
