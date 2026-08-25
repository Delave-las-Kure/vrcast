/**
 * T107 — проверка того, что обязательство GPL выполняется на деле.
 *
 * Проверяется не наличие красивого экрана, а два обещания: сказано, под какой
 * лицензией распространяется приложение, и назван адрес, где взять исходный код
 * ИМЕННО ЭТОЙ версии. Ссылка на «последнюю» версию обязательства не выполняет:
 * человеку на руки досталась конкретная сборка, и право у него на её исходники.
 */

import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderIn } from "../../../test-utils";

const mockAppVersions = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual =
    await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return { ...actual, ipc: { appVersions: () => mockAppVersions() } };
});

const { About } = await import("../About");

beforeEach(() => {
  vi.clearAllMocks();
  mockAppVersions.mockResolvedValue({ app: "0.1.0", server: null, schema: 5 });
});

describe("о программе", () => {
  it("называет лицензию", async () => {
    renderIn(<About />);
    expect(await screen.findByText(/GNU General Public License/)).toBeInTheDocument();
  });

  it("даёт адрес исходного кода ссылкой, по которой можно перейти", async () => {
    renderIn(<About />);
    const ссылка = await screen.findByRole("link", { name: /github\.com/ });
    expect(ссылка).toHaveAttribute("href", expect.stringContaining("github.com"));
  });

  it("привязывает исходный код к той версии, что на руках", async () => {
    // Обязательство GPL — про полученную сборку, а не про «последнюю вообще».
    renderIn(<About />);
    expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
  });

  it("указывает, где перечень чужих работ", async () => {
    renderIn(<About />);
    expect(await screen.findByText("THIRD-PARTY.md")).toBeInTheDocument();
  });

  it("не молчит о лицензии, когда версию узнать не удалось", async () => {
    // Ядро может быть недоступно, но обязательство от этого никуда не девается.
    mockAppVersions.mockRejectedValue(new Error("ядро недоступно"));
    renderIn(<About />);
    expect(await screen.findByText(/GNU General Public License/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /github\.com/ })).toBeInTheDocument();
  });
});
