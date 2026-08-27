/**
 * T330 — маскот с точки зрения человека и машины.
 *
 * Три обещания, и второе из них — единственное, ради чего настройка «выключить» вообще
 * что-то значит:
 *
 * 1. **настроение приходит из настоящих событий задач** (FR-102), а не из своего источника:
 *    разойдясь с экраном задач, маскот махал бы упавшей задаче;
 * 2. **выключённый — не загружается вовсе** (FR-103). Проверяется **отсутствием запроса**, а
 *    не отсутствием картинки: картинки нет и когда она просто не отрисовалась, так что
 *    проверка на картинку прошла бы и на маскоте, который честно скачался и спрятался;
 * 3. **беда важнее успеха**: маскот, показывающий «получилось» поверх упавшей задачи, прячет
 *    ровно то, ради чего на него смотрят.
 */

import { screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { Settings, TaskDoneEvent, TaskProgressEvent } from "../../../shared/contract";

/**
 * Поднято наверх вместе с самой подменой: фабрика `vi.mock` вычисляется раньше обычных
 * объявлений файла, и ссылка на простую переменную из неё не дотянулась бы.
 */
const shared = vi.hoisted(() => ({
  drawingAsked: vi.fn(),
  settingsGet: vi.fn(),
  handlers: {
    progress: (_e: unknown) => {},
    done: (_e: unknown) => {},
  },
}));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      settingsGet: () => shared.settingsGet(),
      settingsSet: async (s: unknown) => s,
    },
    onTaskProgress: (h: (e: unknown) => void) => {
      shared.handlers.progress = h;
      return Promise.resolve(() => {});
    },
    onTaskDone: (h: (e: unknown) => void) => {
      shared.handlers.done = h;
      return Promise.resolve(() => {});
    },
    onViewersUpdate: () => Promise.resolve(() => {}),
  };
});

vi.mock("../MascotDrawing", async () => {
  // Считается **сам факт запроса** модуля. Ленивая загрузка обращается сюда только когда
  // маскот действительно рисуется, так что счётчик и есть ответ на «загрузился ли он».
  shared.drawingAsked();
  return await vi.importActual<typeof import("../MascotDrawing")>("../MascotDrawing");
});

const { Mascot } = await import("../Mascot");
const { SettingsProvider } = await import("../../../app/settings");

const SETTINGS: Settings = {
  viewer_activity_threshold_s: 30,
  geo_refine_outside: false,
  concurrent_heavy_tasks: 1,
  mascot: true,
  animations: true,
  language: null,
  theme: null,
};

function progress(over: Partial<TaskProgressEvent> = {}): TaskProgressEvent {
  return {
    event: "progress",
    id: "t1",
    state: "running",
    progress: 0.5,
    stage: null,
    speed_bps: null,
    eta_s: null,
    ...over,
  };
}

function done(over: Partial<TaskDoneEvent> = {}): TaskDoneEvent {
  return { event: "done", id: "t1", state: "completed", error: null, ...over };
}

function show() {
  return renderIn(
    <SettingsProvider>
      <Mascot />
    </SettingsProvider>,
  );
}

describe("the mascot", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    shared.settingsGet.mockResolvedValue(SETTINGS);
  });

  it("goes to work on a real task event and to worry on a failure", async () => {
    show();
    const drawing = await screen.findByTestId("mascot-drawing");
    expect(drawing).toHaveAttribute("data-mood", "idle");

    shared.handlers.progress(progress());
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "working"),
    );

    shared.handlers.done(done({ state: "failed", error: { code: "FFMPEG_BROKEN", details: [] } }));
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "trouble"),
    );
    // И вслух, а не только цветом: тому, кто слушает экран, картинка не говорит ничего.
    expect(screen.getByTestId("mascot-drawing")).toHaveAttribute(
      "aria-label",
      ru.ui.appearance.mascotTrouble,
    );
  });

  it("is pleased when a task really finishes", async () => {
    show();
    await screen.findByTestId("mascot-drawing");

    shared.handlers.progress(progress());
    shared.handlers.done(done());
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "success"),
    );
  });

  it("does not congratulate a cancelled task", async () => {
    // Человек сам её и отменил. Хвалить его за это нелепо, тревожиться тем более.
    show();
    await screen.findByTestId("mascot-drawing");

    shared.handlers.progress(progress());
    shared.handlers.done(done({ state: "cancelled" }));
    await waitFor(() =>
      expect(screen.getByTestId("mascot-drawing")).toHaveAttribute("data-mood", "idle"),
    );
  });

  it("shows nothing at all when it is turned off", async () => {
    // Здесь только про «не видно». Про «не загружен» — в `mascot-off.test.tsx`, отдельным
    // файлом: в этом реестр модулей уже нагрет проверками выше, и счётчик запросов ответил
    // бы про них, а не про эту проверку. Проверено: в общем файле она проходит всегда.
    shared.settingsGet.mockResolvedValue({ ...SETTINGS, mascot: false });
    show();

    await waitFor(() => expect(shared.settingsGet).toHaveBeenCalled());
    expect(screen.queryByTestId("mascot-slot")).not.toBeInTheDocument();
  });
});
