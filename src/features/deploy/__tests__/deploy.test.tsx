/**
 * T291–T293 — deployment, from a person's side.
 *
 * What is checked is what these screens exist for, not that they render. Three promises:
 * nothing starts until the list of changes has been shown and agreed to; a refusal about the
 * domain says what to go and do rather than "it failed"; and "cannot be done here" does not
 * look like "done" — otherwise a report about a fully deployed server that has neither swap
 * nor tuning would read as a success.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { DeployPreview, DomainAnswer, PlannedStep } from "../../../shared/contract";

const mockDnsCheck = vi.fn<() => Promise<DomainAnswer>>();
const mockPlan = vi.fn<() => Promise<DeployPreview>>();
const mockRun = vi.fn<(...a: unknown[]) => Promise<string>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      dnsCheck: () => mockDnsCheck(),
      deployPlan: () => mockPlan(),
      deployRun: (...a: unknown[]) => mockRun(...a),
    }),
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
    step("Tuning", { Skipped: { why: { NotPossibleHere: { detail: "not in a container" } } } }),
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  mockDnsCheck.mockResolvedValue(DOMAIN_OK);
  mockPlan.mockResolvedValue(PREVIEW);
  mockRun.mockResolvedValue("task-1");
});

describe("deployment", () => {
  it("shows what will be done, and does not start of its own accord", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");

    await waitFor(() => expect(screen.getByText(ru.ui.deploy.willChange)).toBeTruthy());

    // The steps are named one by one, not "a few actions".
    expect(screen.getByText(ru.ui.deploySteps.Swap)).toBeTruthy();
    expect(screen.getByText(ru.ui.deploySteps.DnsCheck)).toBeTruthy();

    // **Nothing has been started.** The screen is open, the plan is shown, there is no task.
    expect(mockRun).not.toHaveBeenCalled();
  });

  it("starts only on agreement, and tells the core the agreement was given", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.agreeAndStart)).toBeTruthy());

    fireEvent.click(screen.getByText(ru.ui.deploy.agreeAndStart));

    await waitFor(() => expect(mockRun).toHaveBeenCalled());
    // The third argument is the confirmation. Sending `false` from here would earn a refusal
    // the person did not deserve — they have just agreed.
    expect(mockRun.mock.calls[0]?.[2]).toBe(true);
  });

  it('does not let "cannot be done here" look like "done"', async () => {
    // Folded into "ready", steps like these produce a report about a fully deployed server
    // that has neither swap nor tuning. Such a report gets believed.
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploySteps.Tuning)).toBeTruthy());

    expect(screen.getByText(ru.ui.deploy.stepNotHere)).toBeTruthy();
    expect(screen.queryAllByText(ru.ui.deploy.stepApplied).length).toBe(1);
  });

  it("stops the start when the domain leads elsewhere, and says what to do", async () => {
    mockDnsCheck.mockResolvedValue(DOMAIN_WRONG);
    renderIn(<DeployScreen serverId="s1" />, "ru");

    // Where it leads now is what a person compares with their registrar's page.
    await waitFor(() => expect(screen.getByText("198.51.100.7")).toBeTruthy());
    expect(screen.getByText(ru.ui.deploy.domainAskAgain)).toBeTruthy();

    // There is no plan at all: a list of changes that cannot be applied reads as an offer,
    // and a person agrees to it for nothing.
    expect(screen.queryByText(ru.ui.deploy.willChange)).toBeNull();
    expect(mockPlan).not.toHaveBeenCalled();
  });

  it("asks about the domain again when asked to — a record takes minutes to travel", async () => {
    mockDnsCheck.mockResolvedValue(DOMAIN_WRONG);
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.domainAskAgain)).toBeTruthy());

    const asked = mockDnsCheck.mock.calls.length;
    fireEvent.click(screen.getByText(ru.ui.deploy.domainAskAgain));
    await waitFor(() => expect(mockDnsCheck.mock.calls.length).toBeGreaterThan(asked));
  });

  it("asks about the domain again when the IPv6 choice changes", async () => {
    // The same domain gives two different verdicts under "keep" and under "turn off". Showing
    // yesterday's is worse than showing none.
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(mockDnsCheck).toHaveBeenCalled());

    const asked = mockDnsCheck.mock.calls.length;
    fireEvent.click(screen.getByLabelText(/IPv6/i, { selector: "input[value='Keep']" }));
    await waitFor(() => expect(mockDnsCheck.mock.calls.length).toBeGreaterThan(asked));
  });
});
