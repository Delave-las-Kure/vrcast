/**
 * T330 — выключённый маскот не загружается вовсе (FR-103).
 *
 * **Отдельный файл, и это не аккуратность.** Модуль, однажды загруженный, остаётся в реестре
 * до конца файла, и счётчик «запрашивался ли рисунок» после первой же проверки, где маскот
 * включён, отвечает про неё, а не про эту. В общем файле такая проверка проходит всегда — я
 * это и получил, прежде чем разнести их. У каждого файла реестр свой, и здесь счётчик
 * означает ровно то, что написано.
 *
 * **Проверяется отсутствием запроса, а не отсутствием картинки.** Картинки нет и у маскота,
 * который честно скачался и спрятался, — а он-то и есть то, что настройка должна была убрать:
 * выключают его на слабой машине, и «не виден» ей ничего не даёт.
 */

import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderIn } from "../../../test-utils";
import type { Settings } from "../../../shared/contract";

const shared = vi.hoisted(() => ({ drawingAsked: vi.fn(), settingsGet: vi.fn() }));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      settingsGet: () => shared.settingsGet(),
      settingsSet: async (s: unknown) => s,
    },
    onTaskProgress: () => Promise.resolve(() => {}),
    onTaskDone: () => Promise.resolve(() => {}),
    onViewersUpdate: () => Promise.resolve(() => {}),
  };
});

vi.mock("../MascotDrawing", async () => {
  shared.drawingAsked();
  return await vi.importActual<typeof import("../MascotDrawing")>("../MascotDrawing");
});

const { Mascot } = await import("../Mascot");
const { SettingsProvider } = await import("../../../app/settings");

const OFF: Settings = {
  viewer_activity_threshold_s: 30,
  geo_refine_outside: false,
  concurrent_heavy_tasks: 1,
  mascot: false,
  animations: true,
  language: null,
  theme: null,
};

describe("a mascot that was turned off", () => {
  it("is never fetched", async () => {
    shared.settingsGet.mockResolvedValue(OFF);
    renderIn(
      <SettingsProvider>
        <Mascot />
      </SettingsProvider>,
    );

    await waitFor(() => expect(shared.settingsGet).toHaveBeenCalled());
    // Ещё немного времени: ленивая загрузка успела бы случиться, если бы началась.
    await new Promise((r) => setTimeout(r, 50));

    expect(shared.drawingAsked).not.toHaveBeenCalled();
    expect(screen.queryByTestId("mascot-slot")).not.toBeInTheDocument();
  });
});
