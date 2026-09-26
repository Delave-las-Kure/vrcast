/**
 * T593 — UpgradeDialog guards its own "agree, upgrade" button against a repeat click,
 * the same way `deploy.test.tsx` guards DeployScreen's `agreeAndStart`.
 *
 * A second `serverUpgradeRun` sent because a click landed before the first one's answer
 * is a second, unasked-for upgrade run on somebody's real server — this is the frontend
 * half of T593, and its only job here is to prove it cannot happen.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { PlannedStep, UpgradePlan } from "../../../shared/contract";

const mockPlan = vi.fn<() => Promise<UpgradePlan>>();
const mockRun = vi.fn<(...a: unknown[]) => Promise<string>>();
const mockRollback = vi.fn<(...a: unknown[]) => Promise<void>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      serverUpgradePlan: () => mockPlan(),
      serverUpgradeRun: (...a: unknown[]) => mockRun(...a),
      serverRollback: (...a: unknown[]) => mockRollback(...a),
    }),
    onTaskDone: () => Promise.resolve(() => {}),
  };
});

const { UpgradeDialog } = await import("../UpgradeDialog");

function step(id: string, status: PlannedStep["status"]): PlannedStep {
  return { id, changes: [], blocking: true, status };
}

const PLAN: UpgradePlan = {
  from: 3,
  to: 4,
  steps: [step("Caddy", "NotApplied")],
  backing_up: ["/etc/caddy/Caddyfile"],
};

beforeEach(() => {
  vi.clearAllMocks();
  mockPlan.mockResolvedValue(PLAN);
  mockRun.mockResolvedValue("task-1");
  mockRollback.mockResolvedValue(undefined);
});

describe("T593 — UpgradeDialog guards agreeAndUpgrade against a repeat click", () => {
  it("sends serverUpgradeRun exactly once for three quick clicks", async () => {
    // Left hanging on purpose: a synchronously-resolving mock could let its own
    // `.finally` clear `starting` between two `fireEvent.click` calls, and the race this
    // test exists to catch would never actually be exercised.
    let resolveRun: (id: string) => void = () => {};
    mockRun.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveRun = resolve;
        }),
    );

    renderIn(<UpgradeDialog serverId="s1" />, "ru");
    const button = await screen.findByText(ru.ui.upgrade.agreeAndUpgrade);
    await waitFor(() => expect(button).toBeEnabled());

    // Three clicks with nothing awaited between them. A real browser already refuses
    // the second and third once `disabled` is set after the first, but `fireEvent.click`
    // in jsdom does not respect that HTML attribute (same note as T589's own test in
    // library.test.tsx) — the handler has to refuse itself, not rely on the markup.
    fireEvent.click(button);
    fireEvent.click(button);
    fireEvent.click(button);

    expect(mockRun).toHaveBeenCalledTimes(1);

    resolveRun("task-1");
    await waitFor(() => expect(mockRun).toHaveBeenCalledTimes(1));
  });
});

describe("T611 — rolling back asks first, and says what it will not put back", () => {
  it("does not roll back at the first click; shows what comes back and what does not", async () => {
    renderIn(<UpgradeDialog serverId="s1" />, "ru");
    const rollBack = await screen.findByText(ru.ui.upgrade.rollBack);
    await waitFor(() => expect(rollBack).toBeEnabled());

    fireEvent.click(rollBack);

    // Nothing sent yet: the first click only opens the question.
    expect(mockRollback).not.toHaveBeenCalled();
    expect(screen.getByText(ru.ui.upgrade.rollBackReturns)).toBeTruthy();
    const keeps = screen.getByText(ru.ui.upgrade.rollBackKeeps);
    // The three things the owner asked to be named (T611): a file the run created, live
    // state, and the limit rules T610 keeps out of the copy.
    expect(keeps.textContent).toContain("99-vrcast-ipv6.conf");
    expect(keeps.textContent).toContain("sysctl");
    expect(keeps.textContent).toContain("ufw");
  });

  it("cancelling sends nothing", async () => {
    renderIn(<UpgradeDialog serverId="s1" />, "ru");
    fireEvent.click(await screen.findByText(ru.ui.upgrade.rollBack));

    const dialog = screen.getByRole("dialog", { name: ru.ui.upgrade.rollBackTitle });
    fireEvent.click(within(dialog).getByText(ru.ui.upgrade.cancel));

    expect(screen.queryByText(ru.ui.upgrade.rollBackKeeps)).toBeNull();
    expect(mockRollback).not.toHaveBeenCalled();
  });

  it("rolls back once on agreement, however many clicks, and says it is done", async () => {
    let resolveRollback: () => void = () => {};
    mockRollback.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          resolveRollback = resolve;
        }),
    );
    renderIn(<UpgradeDialog serverId="s1" />, "ru");
    fireEvent.click(await screen.findByText(ru.ui.upgrade.rollBack));

    const confirm = screen.getByText(ru.ui.upgrade.rollBackConfirm);
    fireEvent.click(confirm);
    fireEvent.click(confirm);

    expect(mockRollback).toHaveBeenCalledTimes(1);
    expect(mockRollback.mock.calls[0]?.[0]).toBe("s1");

    resolveRollback();
    await waitFor(() => expect(screen.getByText(ru.ui.upgrade.rollBackDone)).toBeTruthy());
    expect(screen.queryByText(ru.ui.upgrade.rollBackKeeps)).toBeNull();
  });
});
