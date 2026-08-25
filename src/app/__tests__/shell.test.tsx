/**
 * T028 — тесты оболочки интерфейса.
 *
 * Ядро здесь подменено: тест интерфейса не должен требовать живого приложения, сервера
 * или базы. Проверяется поведение показа — что разделы на месте, что незаконченные
 * помечены честно, что ошибка от ядра доходит до человека неизменной.
 */

import { render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, Task } from "../../shared/contract";

// Подмена обязана быть объявлена до импорта проверяемого кода.
const mockTasksList = vi.fn<() => Promise<Task[]>>();
const mockAppVersions = vi.fn();

vi.mock("../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../shared/ipc")>("../../shared/ipc");
  return {
    ...actual,
    ipc: {
      appVersions: () => mockAppVersions(),
      tasksList: () => mockTasksList(),
      taskGet: vi.fn(),
      taskCancel: vi.fn(),
      taskPause: vi.fn(),
      taskResume: vi.fn(),
      tasksReorder: vi.fn(),
      tasksQueueOrder: vi.fn(),
      tasksOnClose: vi.fn(),
      serverProbeFingerprint: vi.fn(),
    },
    onTaskProgress: vi.fn(async () => () => {}),
    onTaskDone: vi.fn(async () => () => {}),
    onLibraryChanged: vi.fn(async () => () => {}),
  };
});

const { default: App } = await import("../App");
const { ThemeProvider } = await import("../theme");
const { ErrorNotice } = await import("../../features/shared/ErrorNotice");

function makeTask(over: Partial<Task> = {}): Task {
  return {
    id: "t1",
    kind: "upload",
    server_id: null,
    state: "running",
    progress: 0.42,
    stage: "передаём",
    speed_bps: 2_500_000,
    eta_s: 900,
    resume_token: null,
    error: null,
    queue_order: 1,
    created_at: "2026-08-25T10:00:00Z",
    updated_at: "2026-08-25T10:05:00Z",
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockAppVersions.mockResolvedValue({ app: "0.1.0", server: null, schema: 2 });
  mockTasksList.mockResolvedValue([]);
  document.documentElement.dataset.theme = "";
  localStorage.clear();
  // HashRouter хранит адрес в самом окне, и он переживает размонтирование:
  // без сброса следующий тест откроется на разделе, оставшемся от предыдущего.
  window.location.hash = "#/";
});

describe("оболочка", () => {
  it("показывает все разделы приложения", async () => {
    render(<App />);
    // Ищем внутри меню: название открытого раздела встречается ещё и заголовком,
    // и поиск по всей странице нашёл бы два совпадения.
    const nav = await screen.findByRole("navigation", { name: "Разделы" });
    for (const label of [
      "Серверы",
      "Библиотека",
      "Подготовка",
      "Заливка",
      "Качества",
      "Зрители",
      "Ограничения",
      "Диагностика",
      "Задачи",
    ]) {
      expect(within(nav).getByText(label)).toBeInTheDocument();
    }
  });

  it("показывает версию приложения, когда ядро её вернуло", async () => {
    render(<App />);
    expect(await screen.findByText(/версия 0\.1\.0/)).toBeInTheDocument();
  });

  it("не падает, когда версию получить не удалось", async () => {
    // Версия — украшение. Её отсутствие не повод показывать ошибку на весь экран.
    mockAppVersions.mockRejectedValue(new Error("ядро недоступно"));
    render(<App />);
    expect(await screen.findByText("Задачи")).toBeInTheDocument();
    expect(screen.queryByText(/версия/)).not.toBeInTheDocument();
  });

  it("открывает раздел задач по умолчанию", async () => {
    render(<App />);
    expect(
      await screen.findByText(/Задач пока нет/),
    ).toBeInTheDocument();
  });
});

describe("незаконченные разделы", () => {
  it("называют фазу и чем пользоваться до неё", async () => {
    // Пустой экран без объяснения выглядит поломкой, а «скоро будет» ничего не сообщает.
    window.location.hash = "#/upload";
    render(<App />);

    expect(await screen.findByText("Фаза 2")).toBeInTheDocument();
    expect(await screen.findByText(/vrcast-upload/)).toBeInTheDocument();
  });
});

describe("список задач", () => {
  it("показывает задачу с её состоянием и продвижением", async () => {
    mockTasksList.mockResolvedValue([makeTask()]);
    render(<App />);

    expect(await screen.findByText("заливка на сервер")).toBeInTheDocument();
    expect(await screen.findByText("выполняется")).toBeInTheDocument();

    const bar = await screen.findByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "42");
  });

  it("переводит показатели в человеческий вид", async () => {
    mockTasksList.mockResolvedValue([makeTask({ speed_bps: 2_500_000, eta_s: 5400 })]);
    render(<App />);

    // 2 500 000 байт/с — это 20 Мбит/с; показывать байты пользователю бессмысленно.
    expect(await screen.findByText("20.0 Мбит/с")).toBeInTheDocument();
    expect(await screen.findByText(/осталось ~1 ч 30 мин/)).toBeInTheDocument();
  });

  it("предлагает приостановить выполняющуюся и продолжить приостановленную", async () => {
    mockTasksList.mockResolvedValue([
      makeTask({ id: "a", state: "running" }),
      makeTask({ id: "b", state: "paused" }),
    ]);
    render(<App />);

    expect(await screen.findByText("Приостановить")).toBeInTheDocument();
    expect(await screen.findByText("Продолжить")).toBeInTheDocument();
  });

  it("не предлагает действий у завершённой задачи", async () => {
    mockTasksList.mockResolvedValue([makeTask({ state: "completed", progress: 1 })]);
    render(<App />);

    expect(await screen.findByText("завершена")).toBeInTheDocument();
    expect(screen.queryByText("Отменить")).not.toBeInTheDocument();
    expect(screen.queryByText("Приостановить")).not.toBeInTheDocument();
  });

  it("показывает ошибку ядра, когда список прочитать не удалось", async () => {
    const err: AppError = {
      code: "STORAGE_FAILED",
      message: "Не удалось обратиться к локальному хранилищу",
      hint: "Проверьте, что на диске есть место.",
    };
    mockTasksList.mockRejectedValue(err);
    render(<App />);

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(await screen.findByText(err.message)).toBeInTheDocument();
    expect(await screen.findByText(err.hint)).toBeInTheDocument();
  });
});

describe("показ ошибок", () => {
  it("выводит сообщение и подсказку ядра дословно", () => {
    // Интерфейс не сочиняет своих формулировок: иначе одна и та же беда будет
    // объясняться по-разному на разных экранах (FR-105).
    const err: AppError = {
      code: "HOST_KEY_CHANGED",
      message: "Отпечаток сервера изменился",
      hint: "Если сервер не менялся, не подключайтесь: возможна подмена.",
      cause: "ожидался SHA256:aaa, получен SHA256:bbb",
    };
    render(<ErrorNotice error={err} />);

    expect(screen.getByText(err.message)).toBeInTheDocument();
    expect(screen.getByText(err.hint)).toBeInTheDocument();
    expect(screen.getByText(err.cause!)).toBeInTheDocument();
  });
});

describe("оформление", () => {
  it("по умолчанию следует системе", async () => {
    render(
      <ThemeProvider>
        <span>содержимое</span>
      </ThemeProvider>,
    );
    await waitFor(() => {
      expect(["light", "dark"]).toContain(document.documentElement.dataset.theme);
    });
  });

  it("запоминает выбор между запусками", async () => {
    localStorage.setItem("vrcast.theme", "dark");
    render(
      <ThemeProvider>
        <span>содержимое</span>
      </ThemeProvider>,
    );
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("dark");
    });
  });

  it("не падает, когда локальное хранилище недоступно", async () => {
    // В части окружений обращение к нему бросает исключение — приложение
    // всё равно обязано запуститься.
    const spy = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("хранилище недоступно");
    });
    render(
      <ThemeProvider>
        <span>содержимое</span>
      </ThemeProvider>,
    );
    expect(screen.getByText("содержимое")).toBeInTheDocument();
    spy.mockRestore();
  });
});
