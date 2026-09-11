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
import type { DeployPreview, DomainAnswer, PlannedStep, ServerProfile } from "../../../shared/contract";

const mockDnsCheck = vi.fn<() => Promise<DomainAnswer>>();
const mockPlan = vi.fn<() => Promise<DeployPreview>>();
const mockRun = vi.fn<(...a: unknown[]) => Promise<string>>();
const mockServerUpdate = vi.fn<(...a: unknown[]) => Promise<void>>();

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
      serverUpdate: (...a: unknown[]) => mockServerUpdate(...a),
    }),
    onDeployProgress: () => Promise.resolve(() => {}),
    onTaskDone: () => Promise.resolve(() => {}),
  };
});

const { DeployScreen } = await import("../DeployScreen");
const { useServers } = await import("../../servers/store");

/** A profile for `serverId="s1"`, the id every test in this file renders `DeployScreen`
 *  with. Needed only by the T525(3) tests below, which check that a choice on this
 *  screen is written back into the profile. */
function profile(over: Partial<ServerProfile> = {}): ServerProfile {
  return {
    id: "s1",
    name: "Мой сервер",
    host: "203.0.113.10",
    port: 22,
    user: "root",
    auth_kind: "key",
    secret_ref: "server/s1",
    key_path: null,
    domain: "stream.example.com",
    video_dir: "/srv/video",
    cdn_base: null,
    host_fingerprint: "SHA256:x",
    ipv6_mode: null,
    is_active: true,
    ...over,
  };
}


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

function step(
  id: string,
  status: PlannedStep["status"],
  changes: PlannedStep["changes"] = [],
): PlannedStep {
  return { id, changes, blocking: true, status };
}

/** Picks one of the two IPv6 options — nothing is chosen until this runs (T525(1)). */
function chooseIpv6(choice: "Keep" | "Disable") {
  fireEvent.click(screen.getByLabelText(/IPv6/i, { selector: `input[value='${choice}']` }));
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
  mockServerUpdate.mockResolvedValue(undefined);
  useServers.setState({ profiles: [profile()], loading: false, error: null });
});

describe("deployment", () => {
  it("chooses nothing by default, and the start button waits on a choice (T525(1))", async () => {
    // deploy/ipv6.rs, in the core: two paths, and neither of them is a default — a default
    // here would be a silent decision about somebody else's viewers.
    renderIn(<DeployScreen serverId="s1" />, "ru");

    await waitFor(() => expect(screen.getByText(ru.ui.deploy.ipv6NotChosen)).toBeTruthy());
    expect(screen.getByLabelText(/IPv6/i, { selector: "input[value='Keep']" })).not.toBeChecked();
    expect(
      screen.getByLabelText(/IPv6/i, { selector: "input[value='Disable']" }),
    ).not.toBeChecked();

    // Nothing to check the domain against, and nothing to build a plan for, until a choice
    // is made.
    expect(mockDnsCheck).not.toHaveBeenCalled();
    expect(mockPlan).not.toHaveBeenCalled();
    expect(screen.queryByText(ru.ui.deploy.agreeAndStart)).toBeNull();
  });

  it("shows the plan and enables the start button once an IPv6 option is picked", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.ipv6NotChosen)).toBeTruthy());

    chooseIpv6("Disable");

    await waitFor(() => expect(screen.getByText(ru.ui.deploy.agreeAndStart)).toBeEnabled());
  });

  it("shows what will be done, and does not start of its own accord", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");

    await waitFor(() => expect(screen.getByText(ru.ui.deploy.willChange)).toBeTruthy());

    // The steps are named one by one, not "a few actions".
    expect(screen.getByText(ru.ui.deploySteps.Swap)).toBeTruthy();
    expect(screen.getByText(ru.ui.deploySteps.DnsCheck)).toBeTruthy();

    // **Nothing has been started.** The screen is open, the plan is shown, there is no task.
    expect(mockRun).not.toHaveBeenCalled();
  });

  it("starts only on agreement, and tells the core the agreement was given", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");
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
    chooseIpv6("Disable");
    await waitFor(() => expect(screen.getByText(ru.ui.deploySteps.Tuning)).toBeTruthy());

    expect(screen.getByText(ru.ui.deploy.stepNotHere)).toBeTruthy();
    expect(screen.queryAllByText(ru.ui.deploy.stepApplied).length).toBe(1);
  });

  it("stops the start when the domain leads elsewhere, and says what to do", async () => {
    mockDnsCheck.mockResolvedValue(DOMAIN_WRONG);
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");

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
    chooseIpv6("Disable");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.domainAskAgain)).toBeTruthy());

    const asked = mockDnsCheck.mock.calls.length;
    fireEvent.click(screen.getByText(ru.ui.deploy.domainAskAgain));
    await waitFor(() => expect(mockDnsCheck.mock.calls.length).toBeGreaterThan(asked));
  });

  it("asks about the domain again when the IPv6 choice changes", async () => {
    // The same domain gives two different verdicts under "keep" and under "turn off". Showing
    // yesterday's is worse than showing none.
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");
    await waitFor(() => expect(mockDnsCheck).toHaveBeenCalled());

    const asked = mockDnsCheck.mock.calls.length;
    chooseIpv6("Keep");
    await waitFor(() => expect(mockDnsCheck.mock.calls.length).toBeGreaterThan(asked));
  });

  it("saves the IPv6 choice into the profile, so it is remembered next time (T525(3))", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");

    await waitFor(() => expect(mockServerUpdate).toHaveBeenCalled());
    // The first argument is which profile, the second is the full ServerInput with the
    // choice folded in, the third is `null` — the accepted way of saying "leave the secret
    // alone" (EditServerDialog does the same when its secret field is left empty).
    expect(mockServerUpdate.mock.calls[0]?.[0]).toBe("s1");
    expect((mockServerUpdate.mock.calls[0]?.[1] as { ipv6_mode: string }).ipv6_mode).toBe(
      "disable",
    );
    expect(mockServerUpdate.mock.calls[0]?.[2]).toBeNull();
  });

  it("saves the other IPv6 choice with its own value, not always the same one", async () => {
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Keep");

    await waitFor(() => expect(mockServerUpdate).toHaveBeenCalled());
    expect((mockServerUpdate.mock.calls[0]?.[1] as { ipv6_mode: string }).ipv6_mode).toBe("keep");
  });

  it("does not let a failed save of the IPv6 choice block starting the deployment", async () => {
    mockServerUpdate.mockRejectedValue(new Error("offline"));
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");

    // The choice still drives this run of the screen even though saving it failed.
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.agreeAndStart)).toBeEnabled());
  });
});

describe("T590 — DeployScreen resets all local state on a serverId change", () => {
  it("clears a stuck `running` from server A when switching to server B's own screen", async () => {
    useServers.setState({
      profiles: [profile({ id: "s1" }), profile({ id: "s2" })],
      loading: false,
      error: null,
    });

    const { rerender } = renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.agreeAndStart)).toBeEnabled());

    mockRun.mockResolvedValue("task-a");
    fireEvent.click(screen.getByText(ru.ui.deploy.agreeAndStart));
    await waitFor(() => expect(mockRun).toHaveBeenCalled());

    // Server A's deployment is now "running" and has no task:done of its own — it never
    // settles within this test, exactly like a deployment still going on the real core.
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.running)).toBeInTheDocument());

    // The same instance, switched to server B — this is exactly how DeployPage renders a
    // `?server=` change: no `key`, no remount.
    rerender(<DeployScreen serverId="s2" />);

    // B's screen must not inherit A's "in progress": nobody has chosen anything for B yet,
    // let alone started a deployment on it.
    expect(screen.queryByText(ru.ui.deploy.running)).not.toBeInTheDocument();
    expect(screen.getByText(ru.ui.deploy.ipv6NotChosen)).toBeInTheDocument();
  });
});

describe("T592 — DomainCheck ignores a stale dnsCheck answer after a quick re-switch", () => {
  it("keeps the verdict for the current IPv6 choice when an older request answers later", async () => {
    let resolveKeep: (a: DomainAnswer) => void = () => {};
    mockDnsCheck.mockImplementationOnce(
      () =>
        new Promise<DomainAnswer>((resolve) => {
          resolveKeep = resolve;
        }),
    );
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Keep");
    await waitFor(() => expect(mockDnsCheck).toHaveBeenCalledTimes(1));

    // "Keep"'s own dnsCheck is still hanging. Switching to "Disable" fires a second
    // request, which resolves at once (a plain mockResolvedValueOnce, no pending promise).
    mockDnsCheck.mockResolvedValueOnce(DOMAIN_WRONG);
    chooseIpv6("Disable");
    await waitFor(() => expect(mockDnsCheck).toHaveBeenCalledTimes(2));

    // The screen must already show "Disable"'s own verdict — DOMAIN_WRONG.
    await waitFor(() => expect(screen.getByText("198.51.100.7")).toBeInTheDocument());

    // Now "Keep"'s long-overdue answer lands. It must not overwrite "Disable"'s verdict:
    // by the time it resolves, "Keep" is no longer the choice on screen.
    resolveKeep(DOMAIN_OK);
    await waitFor(() => expect(screen.getByText("198.51.100.7")).toBeInTheDocument());
    expect(screen.queryByText(ru.ui.deploy.domainOk)).not.toBeInTheDocument();
  });
});

describe("T593 — DeployScreen guards agreeAndStart against a repeat click", () => {
  it("sends deployRun exactly once for three quick clicks, then recovers normally", async () => {
    // Left hanging: if `mockRun` resolved synchronously, its own `.finally` could clear
    // `starting` between two `fireEvent.click` calls and the race this test exists to
    // catch would never actually happen.
    let resolveRun: (id: string) => void = () => {};
    mockRun.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveRun = resolve;
        }),
    );
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.agreeAndStart)).toBeEnabled());

    const button = screen.getByText(ru.ui.deploy.agreeAndStart);
    // Three clicks with nothing awaited between them — a real browser would already
    // refuse the second and third because the button carries `disabled` after the
    // first, but `fireEvent.click` in jsdom does not respect that HTML attribute (same
    // note as T589's own test in library.test.tsx) and calls `onClick` regardless. The
    // handler has to refuse itself.
    fireEvent.click(button);
    fireEvent.click(button);
    fireEvent.click(button);

    expect(mockRun).toHaveBeenCalledTimes(1);

    resolveRun("task-1");
    await waitFor(() => expect(screen.getByText(ru.ui.deploy.running)).toBeInTheDocument());
    expect(screen.queryByText(ru.ui.deploy.agreeAndStart)).not.toBeInTheDocument();
  });
});

describe("what a step says it will do", () => {
  /**
   * ⚠ **T507, FR-122.** The core has always worked out what each step changes — the packages
   * by name, the ports, the files, the size of the swap file — and `changes` was typed
   * `unknown[]` in the contract and read by nothing at all. The screen showed fifteen general
   * headings, which is the very thing `StepList`'s own comment says a person is not owed.
   *
   * The test that would have caught it has to look at the sentence, not at the shape: every
   * check there was passed a `changes: []` from the builder above, so the field could have
   * held anything or nothing.
   */
  it("names the packages it will install rather than saying only what the step is called", async () => {
    mockPlan.mockResolvedValue({
      ...PREVIEW,
      steps: [
        step("Packages", "NotApplied", [
          { key: "CHANGE_INSTALLS_PACKAGES", params: { names: "ffmpeg, caddy", count: 2 } },
        ]),
      ],
    });
    renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");

    await screen.findByText(ru.ui.deploySteps.Packages);
    expect(
      await screen.findByText(/ffmpeg, caddy/),
      "the plan says which step will run and not what it will do",
    ).toBeInTheDocument();
  });

  it("says nothing at all for a step that changes nothing", async () => {
    // A step that only looks, and one whose changes are empty — keeping IPv6, or swap that is
    // not needed. An empty list under a heading reads as "something, but we are not saying".
    mockPlan.mockResolvedValue({
      ...PREVIEW,
      steps: [step("Ipv6", "Skipped" as unknown as PlannedStep["status"], [])],
    });
    const { container } = renderIn(<DeployScreen serverId="s1" />, "ru");
    chooseIpv6("Disable");

    await screen.findByText(ru.ui.deploySteps.Ipv6);
    expect(container.querySelector(".step__changes")).toBeNull();
  });
});
