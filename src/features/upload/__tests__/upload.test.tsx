/**
 * T103 — тесты интерфейса заливки и очереди.
 *
 * Ядро подменено: тест интерфейса не должен требовать ни сервера, ни базы. Проверяется
 * то, что видит человек, — а главное, разница между вопросом и отказом. Показать
 * нехватку места как предупреждение с кнопкой «всё равно залить» значило бы соврать:
 * места от согласия не появится.
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
  fireEvent.click(await screen.findByText(ru.ui.upload.pickFile));
  await screen.findByDisplayValue("фильм 22.mp4");
}

describe("экран заливки", () => {
  it("подставляет имя в раздаче из имени выбранного файла", async () => {
    // Чаще всего нужно именно оно. Заставлять человека перепечатывать имя руками —
    // лишняя работа и лишний повод для опечатки.
    renderIn(<UploadScreen />);
    await выбрать_файл();
    expect(screen.getByLabelText(ru.ui.upload.fieldName)).toHaveValue("фильм 22.mp4");
  });

  it("не даёт залить, пока файл не выбран", async () => {
    renderIn(<UploadScreen />);
    expect(await screen.findByText(ru.ui.upload.start)).toBeDisabled();
  });

  it("передаёт ядру путь, имя и предел скорости", async () => {
    renderIn(<UploadScreen />);
    await выбрать_файл();

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

  it("говорит, что заливка продолжится после закрытия приложения", async () => {
    // FR-086. Человек не обязан знать, что «в фоне» здесь значит «переживёт
    // закрытие»: об этом надо сказать прямо.
    renderIn(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    expect(await screen.findByText(ru.ui.upload.startedHint)).toBeInTheDocument();
  });

  it("не подключается к серверу без подтверждённого отпечатка", async () => {
    mockServersList.mockResolvedValue([профиль({ host_fingerprint: null })]);
    renderIn(<UploadScreen />);
    expect(
      await screen.findByText(fill(ru.ui.upload.notReady, { name: "Боевой" }, ru, "ru")),
    ).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.upload.start)).not.toBeInTheDocument();
  });
});

describe("предупреждения до старта", () => {
  const занятое_имя: AppError = {
    code: "NAME_EXISTS",
    details: [{ key: "NAME_WILL_BE_REPLACED", params: { name: "фильм 22.mp4" } }],
  };
  const занятое_имя_словами = fill(
    ru.details.NAME_WILL_BE_REPLACED,
    { name: "фильм 22.mp4" },
    ru,
    "ru",
  );

  const нет_места: AppError = {
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

  it("занятое имя показывается вопросом и снимается согласием", async () => {
    mockUploadStart.mockRejectedValueOnce(занятое_имя);
    renderIn(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    expect(await screen.findByText(занятое_имя_словами)).toBeInTheDocument();

    // Согласие уходит в ядро тем же запросом, но уже подтверждённым.
    mockUploadStart.mockResolvedValueOnce("t-2");
    fireEvent.click(screen.getByText(ru.ui.preflight.uploadAnyway));

    await waitFor(() => expect(mockUploadStart).toHaveBeenCalledTimes(2));
    expect(mockUploadStart).toHaveBeenLastCalledWith(
      expect.objectContaining({ confirmed: true }),
    );
  });

  it("нехватка места согласием не снимается", async () => {
    // Ради этого различия компонент и существует. Кнопка «всё равно залить» здесь
    // была бы обманом: передача упрётся в конец диска на середине.
    mockUploadStart.mockRejectedValue(нет_места);
    renderIn(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    // The numbers are the core's; the units and the separator are the language's.
    const сказано = await screen.findByText(/На сервере не хватает 22,0 ГБ/);
    expect(сказано).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.preflight.uploadAnyway)).not.toBeInTheDocument();
  });

  it("предупреждение показывается до начала передачи, а не после", async () => {
    // Задача не должна была поставиться: узнать о занятом имени после часа
    // передачи — то же самое, что не предупреждать вовсе.
    mockUploadStart.mockRejectedValue(занятое_имя);
    renderIn(<UploadScreen />);
    await выбрать_файл();
    fireEvent.click(screen.getByText(ru.ui.upload.start));

    await screen.findByText(занятое_имя_словами);
    expect(screen.queryByText(ru.ui.upload.started)).not.toBeInTheDocument();
  });

  it("говорит о нехватке места по-английски, когда выбран английский", async () => {
    mockUploadStart.mockRejectedValue(нет_места);
    renderIn(<UploadScreen />, "en");
    fireEvent.click(await screen.findByText(en.ui.upload.pickFile));
    await screen.findByDisplayValue("фильм 22.mp4");
    fireEvent.click(screen.getByText(en.ui.upload.start));

    expect(await screen.findByText(/The server is 22\.0 GB short/)).toBeInTheDocument();
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
    renderIn(
      <QueueOrder
        queued={[задача("a", 1), задача("b", 2), задача("c", 3)]}
        busy={false}
        onReorder={onReorder}
      />,
    );

    // Поднимаем третью.
    fireEvent.click(screen.getAllByLabelText(ru.ui.tasks.moveUp)[2]);
    expect(onReorder).toHaveBeenCalledWith(["a", "c", "b"]);
  });

  it("первую поднять некуда, последнюю опустить некуда", () => {
    renderIn(
      <QueueOrder
        queued={[задача("a", 1), задача("b", 2)]}
        busy={false}
        onReorder={vi.fn()}
      />,
    );
    expect(screen.getAllByLabelText(ru.ui.tasks.moveUp)[0]).toBeDisabled();
    expect(screen.getAllByLabelText(ru.ui.tasks.moveDown)[1]).toBeDisabled();
  });

  it("пустая очередь не показывается вовсе", () => {
    const { container } = renderIn(
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
    explanation: { key: "ON_CLOSE_RESUMES_FROM", params: { percent: 60 } },
  };
  const заново: TaskOnClose = {
    id: "b",
    kind: "convert",
    progress: 0.4,
    outcome: "restarts",
    explanation: { key: "ON_CLOSE_RESTARTS_LOSING", params: { percent: 40 } },
  };

  it("предупреждает, когда работа потеряется", () => {
    renderIn(<CloseConsequences items={[продолжится, заново]} />);
    expect(screen.getByText(ru.ui.tasks.closeLosing)).toBeInTheDocument();
    expect(
      screen.getByText(
        fill(ru.details.ON_CLOSE_RESTARTS_LOSING, { percent: 40 }, ru, "ru"),
      ),
    ).toBeInTheDocument();
  });

  it("успокаивает, когда всё продолжится", () => {
    renderIn(<CloseConsequences items={[продолжится]} />);
    expect(screen.getByText(ru.ui.tasks.closeSafe)).toBeInTheDocument();
  });

  it("молчит, когда закрывать безопасно и говорить не о чем", () => {
    const { container } = renderIn(<CloseConsequences items={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
