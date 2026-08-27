/**
 * T291–T293 — развёртывание с точки зрения человека.
 *
 * Проверяется то, ради чего эти экраны существуют, а не то, что они рисуются. Три обещания:
 * ничего не начинается, пока список изменений не показан и с ним не согласились; отказ по
 * домену говорит, что пойти и сделать, а не «не удалось»; и «здесь не установить» не
 * выглядит как «сделано» — иначе отчёт о полностью развёрнутом сервере, у которого нет ни
 * подкачки, ни тюнинга, читался бы как успех.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { DeployPreview, DomainAnswer, PlannedStep } from "../../../shared/contract";

const mockDnsCheck = vi.fn<() => Promise<DomainAnswer>>();
const mockPlan = vi.fn<() => Promise<DeployPreview>>();
const mockRun = vi.fn<(...a: unknown[]) => Promise<string>>();

vi.mock("../../../shared/ipc", async () => {
  const actual =
    await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      dnsCheck: () => mockDnsCheck(),
      deployPlan: () => mockPlan(),
      deployRun: (...a: unknown[]) => mockRun(...a),
    },
    onDeployProgress: () => Promise.resolve(() => {}),
    onTaskDone: () => Promise.resolve(() => {}),
  };
});

const { DeployScreen } = await import("../DeployScreen");

const DOMAIN_OK: DomainAnswer = { verdict: "Ok", a: ["203.0.113.10"], aaaa: [], advice: null };

const DOMAIN_WRONG: DomainAnswer = {
  verdict: { PointsElsewhere: { record: "A", to: ["198.51.100.7"] } },
  a: ["198.51.100.7"],
  aaaa: [],
  advice: {
    key: "DOMAIN_FIX_RECORD",
    params: {
      record: "A",
      name: "stream.example.com",
      to: "198.51.100.7",
      value: "203.0.113.10",
    },
  },
};

function step(id: string, status: PlannedStep["status"]): PlannedStep {
  return { id, changes: [], blocking: true, status };
}

const PREVIEW: DeployPreview = {
  domain: DOMAIN_OK,
  memory_mb: 961,
  disk: "vda",
  steps: [
    step("DnsCheck", "Applied"),
    step("Swap", "NotApplied"),
    step("Tuning", { Skipped: { why: { NotPossibleHere: { detail: "в контейнере нельзя" } } } }),
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  mockDnsCheck.mockResolvedValue(DOMAIN_OK);
  mockPlan.mockResolvedValue(PREVIEW);
  mockRun.mockResolvedValue("task-1");
});

describe("развёртывание", () => {
  it("показывает, что будет сделано, и не начинает само", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");

    await waitFor(() => expect(screen.getByText(ru.ui.deploy.willChange)).toBeTruthy());

    // Шаги названы поимённо, а не «несколько действий».
    expect(screen.getByText(ru.ui.deploySteps.Swap)).toBeTruthy();
    expect(screen.getByText(ru.ui.deploySteps.DnsCheck)).toBeTruthy();

    // **Ничего не запущено.** Экран открыт, план показан, задачи нет.
    expect(mockRun).not.toHaveBeenCalled();
  });

  it("начинает только по согласию, и говорит серверу, что согласие есть", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.agreeAndStart)).toBeTruthy());

    fireEvent.click(screen.getByText(ru.ui.deploy.agreeAndStart));

    await waitFor(() => expect(mockRun).toHaveBeenCalled());
    // Третьим доводом идёт подтверждение. Отправить `false` отсюда значило бы получить
    // отказ, которого человек не заслужил, — он только что согласился.
    expect(mockRun.mock.calls[0]?.[2]).toBe(true);
  });

  it("«здесь не установить» не выглядит как «сделано»", async () => {
    // Свёрнутые в «готово», такие шаги дают отчёт о полностью развёрнутом сервере, у
    // которого нет ни подкачки, ни тюнинга. Такому отчёту верят.
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploySteps.Tuning)).toBeTruthy());

    expect(screen.getByText(ru.ui.deploy.stepNotHere)).toBeTruthy();
    expect(screen.queryAllByText(ru.ui.deploy.stepApplied).length).toBe(1);
  });

  it("домен, ведущий не сюда, останавливает начало и говорит, что делать", async () => {
    mockDnsCheck.mockResolvedValue(DOMAIN_WRONG);
    renderIn(<DeployScreen serverId="s1" />, "ru");

    // Куда ведёт сейчас — это то, что человек сверяет со страницей регистратора.
    await waitFor(() => expect(screen.getByText("198.51.100.7")).toBeTruthy());
    expect(screen.getByText(ru.ui.deploy.domainAskAgain)).toBeTruthy();

    // Плана нет вовсе: список изменений, который нельзя применить, читается как
    // предложение, и человек согласится с ним впустую.
    expect(screen.queryByText(ru.ui.deploy.willChange)).toBeNull();
    expect(mockPlan).not.toHaveBeenCalled();
  });

  it("спрашивает домен заново по просьбе — запись расходится минутами", async () => {
    mockDnsCheck.mockResolvedValue(DOMAIN_WRONG);
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.domainAskAgain)).toBeTruthy());

    const asked = mockDnsCheck.mock.calls.length;
    fireEvent.click(screen.getByText(ru.ui.deploy.domainAskAgain));
    await waitFor(() => expect(mockDnsCheck.mock.calls.length).toBeGreaterThan(asked));
  });

  it("смена выбора про IPv6 спрашивает домен заново", async () => {
    // Тот же домен при «оставить» и при «отключить» — два разных вердикта. Показывать
    // вчерашний хуже, чем не показывать никакого.
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(mockDnsCheck).toHaveBeenCalled());

    const asked = mockDnsCheck.mock.calls.length;
    fireEvent.click(screen.getByLabelText(/IPv6/i, { selector: "input[value='Keep']" }));
    await waitFor(() => expect(mockDnsCheck.mock.calls.length).toBeGreaterThan(asked));
  });
});
