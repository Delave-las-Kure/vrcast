/**
 * T593 — UpgradeDialog guards its own "agree, upgrade" button against a repeat click,
 * the same way `deploy.test.tsx` guards DeployScreen's `agreeAndStart`.
 *
 * A second `serverUpgradeRun` sent because a click landed before the first one's answer
 * is a second, unasked-for upgrade run on somebody's real server — this is the frontend
 * half of T593, and its only job here is to prove it cannot happen.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { PlannedStep, UpgradePlan } from "../../../shared/contract";

const mockPlan = vi.fn<() => Promise<UpgradePlan>>();
const mockRun = vi.fn<(...a: unknown[]) => Promise<string>>();

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
