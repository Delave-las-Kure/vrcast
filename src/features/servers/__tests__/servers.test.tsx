/**
 * T059 — the servers section.
 *
 * The core is stood in for: a check of the interface must need neither a server nor a
 * database. What is checked is what a person gets burned by in life — that confirming a
 * fingerprint cannot be skipped, that every step of a check is on screen and not only the
 * one that broke, and that a profile without a confirmed fingerprint does not look ready.
 *
 * **The Cyrillic that is left is deliberate.** A profile named `Мой сервер` and a serving
 * directory at `/srv/раздача/видео` are what half this project's users will really have,
 * and a check that only ever sees Latin names proves nothing about them.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { en, renderIn, ru } from "../../../test-utils";
import { fill } from "../../../shared/i18n/render";
import type { ServerProfile, ServerState, TestStep } from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockServerAdd = vi.fn();
const mockServerUpdate = vi.fn();
const mockServerTest = vi.fn<() => Promise<TestStep[]>>();
const mockProbeFingerprint = vi.fn<() => Promise<string>>();
const mockConfirmFingerprint = vi.fn();
const mockServerRemove = vi.fn();
const mockSetActive = vi.fn();
const mockServerDetect = vi.fn<() => Promise<ServerState>>(() =>
  Promise.reject({ code: "SSH_UNREACHABLE" }),
);
const mockImportSuggestion = vi.fn();

/** What the core would send over `server:state`. Held so a test can push an update
 *  whenever it likes, the same way the viewers screen's test captures `onViewersUpdate`. */
let sendServerState: ((serverId: string, state: ServerState) => void) | null = null;
const unlistenServerState = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      serversList: () => mockServersList(),
      serverAdd: (...a: unknown[]) => mockServerAdd(...a),
      serverUpdate: (...a: unknown[]) => mockServerUpdate(...a),
      serverRemove: (...a: unknown[]) => mockServerRemove(...a),
      serverSetActive: (...a: unknown[]) => mockSetActive(...a),
      serverTest: (...a: unknown[]) => mockServerTest(...(a as [])),
      serverFingerprintConfirm: (...a: unknown[]) => mockConfirmFingerprint(...a),
      serverProbeFingerprint: (...a: unknown[]) => mockProbeFingerprint(...(a as [])),
      serverImportSuggestion: () => mockImportSuggestion(),
      // Since T294 the server card asks what kind of server this is. Here it does not answer
      // — and that state is a real one: a silent server must not bring the list down.
      serverDetect: () => mockServerDetect(),
    }),
    onServerState: vi.fn(async (handler: (serverId: string, state: ServerState) => void) => {
      sendServerState = handler;
      return unlistenServerState;
    }),
  };
});

const { ServerList } = await import("../ServerList");
const { ServerStateCard } = await import("../ServerStateCard");
const { useServers } = await import("../store");

function makeProfile(over: Partial<ServerProfile> = {}): ServerProfile {
  return {
    id: "srv_1",
    name: "Мой сервер",
    host: "203.0.113.10",
    port: 22,
    user: "root",
    auth_kind: "key",
    secret_ref: "server/srv_1",
    key_path: "/home/u/.ssh/id_ed25519",
    domain: "stream.example.com",
    video_dir: "/srv/раздача/видео",
    cdn_base: null,
    host_fingerprint: "SHA256:тестовыйОтпечаток",
    ipv6_mode: null,
    is_active: true,
    ...over,
  };
}

function steps(): TestStep[] {
  return [
    {
      id: "network",
      status: "failed",
      detail: { key: "STEP_NET_TIMEOUT", params: { seconds: 10 } },
    },
    { id: "login", status: "skipped", detail: null },
    { id: "video_dir", status: "skipped", detail: null },
    { id: "domain", status: "skipped", detail: null },
  ];
}

const draw = (lang: "ru" | "en" = "ru") =>
  renderIn(
    <MemoryRouter>
      <ServerList />
    </MemoryRouter>,
    lang,
  );

beforeEach(() => {
  vi.clearAllMocks();
  // The store is shared across the module and outlives unmounting: without a reset the
  // next test sees the previous one's profiles.
  useServers.setState({ profiles: [], loading: true, error: null });
  mockServersList.mockResolvedValue([]);
  mockImportSuggestion.mockResolvedValue(null);
  sendServerState = null;
});

describe("the list of servers", () => {
  it("explains the emptiness rather than showing an empty screen", async () => {
    draw();
    expect(await screen.findByText(ru.ui.servers.empty)).toBeInTheDocument();
  });

  it("shows the server's address and domain", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    draw();

    expect(await screen.findByText("Мой сервер")).toBeInTheDocument();
    expect(screen.getByText("root@203.0.113.10")).toBeInTheDocument();
    expect(screen.getByText("stream.example.com")).toBeInTheDocument();
  });

  it("marks a profile whose fingerprint has not been confirmed", async () => {
    // Such a profile exists and cannot be connected with. Saying nothing about that
    // leaves a person guessing why nothing works.
    mockServersList.mockResolvedValue([makeProfile({ host_fingerprint: null })]);
    draw();

    expect(await screen.findByText(ru.ui.servers.fingerprintUnconfirmed)).toBeInTheDocument();
  });

  it("does not ask about removal blindly", async () => {
    // FR-005: the way into the server is forgotten along with the profile. That has to be
    // said before the button, not after it.
    mockServersList.mockResolvedValue([makeProfile()]);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.remove));
    expect(await screen.findByText(ru.ui.servers.confirmRemoval)).toBeInTheDocument();
    expect(mockServerRemove).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(ru.ui.servers.removeYes));
    await waitFor(() => expect(mockServerRemove).toHaveBeenCalledWith("srv_1"));
  });

  it("shows every step of the check, including the ones not run", async () => {
    // FR-003. A person needs to see what got through, not only the last misfortune.
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerTest.mockResolvedValue(steps());
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.test));

    // The title comes from the step's id now, not from the core.
    expect(await screen.findByText(ru.ui.servers.steps.network)).toBeInTheDocument();
    expect(
      screen.getByText(fill(ru.details.STEP_NET_TIMEOUT, { seconds: 10 }, ru, "ru")),
    ).toBeInTheDocument();
    // The steps after the broken one are on screen too, with a note saying why they
    // were not looked at.
    expect(screen.getByText(ru.ui.servers.steps.login)).toBeInTheDocument();
    expect(screen.getByText(ru.ui.servers.steps.domain)).toBeInTheDocument();
    expect(screen.getAllByText(ru.ui.wizard.stepSkipped).length).toBe(3);
  });

  it("shows the same check in English when English is chosen", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerTest.mockResolvedValue(steps());
    draw("en");

    fireEvent.click(await screen.findByText(en.ui.servers.test));

    expect(await screen.findByText(en.ui.servers.steps.network)).toBeInTheDocument();
    expect(
      screen.getByText(fill(en.details.STEP_NET_TIMEOUT, { seconds: 10 }, en, "en")),
    ).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.servers.steps.network)).not.toBeInTheDocument();
  });
});

describe("the setup wizard", () => {
  it("requires the fingerprint to be confirmed before anything is tried", async () => {
    // The one step that cannot be skipped: until it is confirmed the application sends the
    // server neither a password nor a key (FR-092).
    mockServerAdd.mockResolvedValue("srv_new");
    mockProbeFingerprint.mockResolvedValue("SHA256:ОтпечатокНовогоСервера");
    mockServerTest.mockResolvedValue(steps());
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.add));

    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldName), { target: { value: "Тест" } });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldHost), {
      target: { value: "203.0.113.10" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldDomain), {
      target: { value: "stream.example.com" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldKeyPath), {
      target: { value: "/home/u/.ssh/k" },
    });
    fireEvent.click(screen.getByText(ru.ui.wizard.next));

    // The fingerprint is on screen and nothing has been tried yet.
    expect(await screen.findByText("SHA256:ОтпечатокНовогоСервера")).toBeInTheDocument();
    expect(mockServerTest).not.toHaveBeenCalled();
    expect(mockConfirmFingerprint).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(ru.ui.wizard.fingerprintOk));
    await waitFor(() =>
      expect(mockConfirmFingerprint).toHaveBeenCalledWith(
        "srv_new",
        "SHA256:ОтпечатокНовогоСервера",
      ),
    );
    await waitFor(() => expect(mockServerTest).toHaveBeenCalled());
  });

  it("clears the profile away when the person declines at the fingerprint step", async () => {
    // Otherwise a half-made server stays in the list, cannot be connected with, and the
    // person has no idea where it came from.
    mockServerAdd.mockResolvedValue("srv_new");
    mockProbeFingerprint.mockResolvedValue("SHA256:чужой");
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.add));
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldName), { target: { value: "Тест" } });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldHost), {
      target: { value: "203.0.113.10" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldDomain), {
      target: { value: "stream.example.com" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldKeyPath), {
      target: { value: "/home/u/.ssh/k" },
    });
    fireEvent.click(screen.getByText(ru.ui.wizard.next));

    fireEvent.click(await screen.findByText(ru.ui.wizard.abandon));
    await waitFor(() => expect(mockServerRemove).toHaveBeenCalledWith("srv_new"));
  });

  it("offers to carry settings over when the old file is found beside it", async () => {
    mockImportSuggestion.mockResolvedValue({
      source: "F:\\Stream Server\\server.env",
      needs_passphrase: true,
      input: {
        name: "stream.example.com",
        host: "203.0.113.10",
        port: 22,
        user: "root",
        auth_kind: "key",
        key_path: "/home/u/.ssh/vrcast",
        domain: "stream.example.com",
        video_dir: null,
        cdn_base: null,
        ipv6_mode: null,
      },
    });
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.add));
    expect(await screen.findByText(ru.ui.wizard.importFound)).toBeInTheDocument();
    // The passphrase is spoken of honestly: it is not in the file and cannot be.
    expect(
      screen.getByText(new RegExp(ru.ui.wizard.importNeedsPassphrase.trim())),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByText(ru.ui.wizard.importApply));
    await waitFor(() =>
      expect(screen.getByLabelText(ru.ui.wizard.fieldHost)).toHaveValue("203.0.113.10"),
    );
  });
});

describe("editing a server profile", () => {
  it("opens the edit form filled with the profile's current values", async () => {
    // The secret is never sent back from the core: the passphrase field must stay empty
    // rather than showing something that only looks like the saved one.
    mockServersList.mockResolvedValue([makeProfile()]);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));

    expect(await screen.findByLabelText(ru.ui.wizard.fieldName)).toHaveValue("Мой сервер");
    expect(screen.getByLabelText(ru.ui.wizard.fieldHost)).toHaveValue("203.0.113.10");
    expect(screen.getByLabelText(ru.ui.wizard.fieldDomain)).toHaveValue("stream.example.com");
    expect(screen.getByLabelText(ru.ui.wizard.fieldKeyPath)).toHaveValue(
      "/home/u/.ssh/id_ed25519",
    );
    expect(screen.getByLabelText(ru.ui.wizard.fieldPassphrase)).toHaveValue("");
  });

  it("saves without re-probing the fingerprint when the address did not change", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldName), {
      target: { value: "Мой сервер 2" },
    });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ name: "Мой сервер 2", host: "203.0.113.10", port: 22 }),
        null,
      ),
    );
    // The fingerprint from before is still confirmed — the core guarantees that — so
    // re-probing it would ask a question already answered.
    expect(mockProbeFingerprint).not.toHaveBeenCalled();
  });

  it("re-probes and requires confirming the fingerprint when the host changes", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerUpdate.mockResolvedValue(undefined);
    mockProbeFingerprint.mockResolvedValue("SHA256:НовыйАдрес");
    mockServerTest.mockResolvedValue(steps());
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldHost), {
      target: { value: "203.0.113.99" },
    });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ host: "203.0.113.99" }),
        null,
      ),
    );

    // The new address's fingerprint is on screen, and nothing has connected yet.
    expect(await screen.findByText("SHA256:НовыйАдрес")).toBeInTheDocument();
    expect(mockConfirmFingerprint).not.toHaveBeenCalled();
    expect(mockServerTest).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(ru.ui.wizard.fingerprintOk));
    await waitFor(() =>
      expect(mockConfirmFingerprint).toHaveBeenCalledWith("srv_1", "SHA256:НовыйАдрес"),
    );
    await waitFor(() => expect(mockServerTest).toHaveBeenCalledWith("srv_1"));
  });
});

function managedState(over: Partial<ServerState> = {}): ServerState {
  return {
    kind: "Managed",
    server_version: 3,
    app_expects: 3,
    app_min_supported: 1,
    compat: "Ok",
    upgrade_available: false,
    foreign_reason: null,
    ...over,
  };
}

describe("the server state card (T538)", () => {
  it("subscribes to server:state on mount and unsubscribes on unmount", async () => {
    mockServerDetect.mockResolvedValue(managedState());
    const view = renderIn(<ServerStateCard serverId="srv_1" />, "ru");

    await waitFor(() => expect(sendServerState).not.toBeNull());
    expect(unlistenServerState).not.toHaveBeenCalled();

    view.unmount();
    await waitFor(() => expect(unlistenServerState).toHaveBeenCalled());
  });

  it("updates the shown state when the event's server_id matches", async () => {
    mockServerDetect.mockResolvedValue(managedState({ server_version: 3, app_expects: 3 }));
    renderIn(<ServerStateCard serverId="srv_1" />, "ru");

    expect(await screen.findByText(ru.ui.serverState.versions(3, 3))).toBeInTheDocument();

    sendServerState?.("srv_1", managedState({ server_version: 4, app_expects: 4 }));

    expect(await screen.findByText(ru.ui.serverState.versions(4, 4))).toBeInTheDocument();
  });

  it("ignores a server:state event for a different server_id", async () => {
    mockServerDetect.mockResolvedValue(managedState({ server_version: 3, app_expects: 3 }));
    renderIn(<ServerStateCard serverId="srv_1" />, "ru");

    expect(await screen.findByText(ru.ui.serverState.versions(3, 3))).toBeInTheDocument();

    sendServerState?.("srv_OTHER", managedState({ server_version: 9, app_expects: 9 }));

    // Given time to (not) re-render, the card still shows its own server's numbers.
    await waitFor(() =>
      expect(screen.getByText(ru.ui.serverState.versions(3, 3))).toBeInTheDocument(),
    );
    expect(screen.queryByText(ru.ui.serverState.versions(9, 9))).not.toBeInTheDocument();
  });
});
