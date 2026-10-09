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

  it("shows a profile with a managed key the same way as any other (T548)", async () => {
    // AuthKind grew a third value the interface never lets a person pick by hand
    // (ServerForm's <select> still offers only "key"/"password") but that a
    // deployment can assign on its own. The card must not choke on it or show
    // "undefined" where the auth kind would otherwise be read.
    mockServersList.mockResolvedValue([makeProfile({ auth_kind: "managed_key", key_path: null })]);
    draw();

    expect(await screen.findByText("Мой сервер")).toBeInTheDocument();
    expect(screen.getByText("root@203.0.113.10")).toBeInTheDocument();
    expect(screen.getByText("stream.example.com")).toBeInTheDocument();
    expect(screen.queryByText("undefined")).not.toBeInTheDocument();
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
    await waitFor(() => expect(mockServerRemove).toHaveBeenCalledWith("srv_1", false));
  });

  it("asks in one line to stop the work on the server before deleting it (T683)", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerRemove.mockImplementation((_id: string, confirmed: boolean) =>
      confirmed
        ? Promise.resolve(undefined)
        : Promise.reject({
            code: "CONFIRMATION_REQUIRED",
            details: [{ key: "CONFIRM_STOP_SERVER_WORK", params: { count: 3 } }],
          }),
    );
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.remove));
    fireEvent.click(screen.getByText(ru.ui.servers.removeYes));
    // The core said there is work: the question is the one line, not an error.
    expect(
      await screen.findByText("Остановить 3 задачи на этом сервере и удалить его?"),
    ).toBeInTheDocument();
    expect(screen.queryByText(ru.errors.CONFIRMATION_REQUIRED.message)).toBeNull();
    expect(mockServerRemove).toHaveBeenCalledTimes(1);
    expect(mockServerRemove).toHaveBeenLastCalledWith("srv_1", false);

    // «Yes» stops it all and deletes.
    fireEvent.click(screen.getByText(ru.ui.servers.removeYes));
    await waitFor(() => expect(mockServerRemove).toHaveBeenLastCalledWith("srv_1", true));
  });

  it("asks nothing more when «Cancel» is pressed at the question (T683)", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerRemove.mockRejectedValue({
      code: "CONFIRMATION_REQUIRED",
      details: [{ key: "CONFIRM_STOP_SERVER_WORK", params: { count: 1 } }],
    });
    draw();
    fireEvent.click(await screen.findByText(ru.ui.servers.remove));
    fireEvent.click(screen.getByText(ru.ui.servers.removeYes));
    expect(
      await screen.findByText("Остановить 1 задачу на этом сервере и удалить его?"),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByText(ru.ui.common.cancel));
    expect(screen.queryByText(/Остановить 1 задачу/)).toBeNull();
    expect(mockServerRemove).toHaveBeenCalledTimes(1);
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

  /** Fill the wizard's form the way a person would and press «Next». */
  const fillAndNext = async () => {
    fireEvent.click(await screen.findByText(ru.ui.servers.add));
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldName), {
      target: { value: "Контейнер" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldHost), {
      target: { value: "127.0.0.1" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldPort), {
      target: { value: "47099" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldDomain), {
      target: { value: "stream.example.com" },
    });
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldKeyPath), {
      target: { value: "/home/u/.ssh/k" },
    });
    fireEvent.click(screen.getByText(ru.ui.wizard.next));
  };

  it("checks the same profile again after the port is fixed, rather than making a second (T708)", async () => {
    mockServerAdd.mockResolvedValue("srv_new");
    mockServerUpdate.mockResolvedValue(undefined);
    mockProbeFingerprint
      .mockRejectedValueOnce({
        code: "SSH_UNREACHABLE",
        details: [{ key: "SSH_PORT_REFUSED", params: { port: 47099 } }],
        cause: "server 127.0.0.1:47099 is unreachable: IO error (os error 10061)",
      })
      .mockResolvedValueOnce("SHA256:верный");
    draw();
    await fillAndNext();

    // One plain line in sight; the system's words only under «Details».
    expect(await screen.findByText("Сервер не отвечает на порту 47099")).toBeInTheDocument();
    expect(screen.getByText(/os error 10061/).closest("details")).not.toBeNull();

    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldPort), {
      target: { value: "47022" },
    });
    fireEvent.click(screen.getByText(ru.ui.wizard.next));

    expect(await screen.findByText("SHA256:верный")).toBeInTheDocument();
    expect(mockServerAdd).toHaveBeenCalledTimes(1);
    expect(mockServerUpdate).toHaveBeenCalledTimes(1);
    const [id, input, secret] = mockServerUpdate.mock.calls[0];
    expect(id).toBe("srv_new");
    expect((input as { port: number }).port).toBe(47022);
    // The field was left empty: the passphrase given the first time is kept.
    expect(secret).toBeNull();
  });

  it("lets a wrong passphrase be fixed in the wizard and checks again without a second fingerprint (T708)", async () => {
    mockServerAdd.mockResolvedValue("srv_new");
    mockServerUpdate.mockResolvedValue(undefined);
    mockProbeFingerprint.mockResolvedValue("SHA256:верный");
    mockServerTest
      .mockResolvedValueOnce([
        {
          id: "network",
          status: "ok",
          detail: { key: "STEP_NET_BANNER", params: { banner: "SSH-2.0-OpenSSH_9.6p1" } },
        },
        { id: "login", status: "failed", detail: { key: "STEP_LOGIN_WRONG_PASSPHRASE" } },
        { id: "video_dir", status: "skipped", detail: null },
        { id: "domain", status: "skipped", detail: null },
      ])
      .mockResolvedValueOnce(steps());
    draw();
    await fillAndNext();
    fireEvent.click(await screen.findByText(ru.ui.wizard.fingerprintOk));

    expect(await screen.findByText(ru.details.STEP_LOGIN_WRONG_PASSPHRASE)).toBeInTheDocument();
    // What the server called itself is folded away, not on the line.
    expect(screen.getByText("SSH-2.0-OpenSSH_9.6p1").closest("details")).not.toBeNull();

    fireEvent.click(screen.getByText(ru.ui.wizard.fixDetails));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldPassphrase), {
      target: { value: "верная фраза" },
    });
    fireEvent.click(screen.getByText(ru.ui.wizard.next));

    await waitFor(() => expect(mockServerTest).toHaveBeenCalledTimes(2));
    expect(mockServerUpdate).toHaveBeenCalledWith("srv_new", expect.anything(), "верная фраза");
    expect(mockProbeFingerprint).toHaveBeenCalledTimes(1);
    expect(mockServerAdd).toHaveBeenCalledTimes(1);
  });

  it("shows the profile in the list at once after «Cancel» (T708)", async () => {
    mockServerAdd.mockResolvedValue("srv_new");
    mockProbeFingerprint.mockRejectedValue({ code: "SSH_UNREACHABLE" });
    draw();
    await fillAndNext();
    await screen.findByText(ru.errors.SSH_UNREACHABLE.message);

    mockServersList.mockResolvedValue([
      makeProfile({ id: "srv_new", name: "Контейнер", host_fingerprint: null }),
    ]);
    fireEvent.click(screen.getByText(ru.ui.common.cancel));
    expect(await screen.findByText("Контейнер")).toBeInTheDocument();
  });

  it("does not make a newly added server active by itself at «Done» (T708)", async () => {
    mockServerAdd.mockResolvedValue("srv_new");
    mockProbeFingerprint.mockResolvedValue("SHA256:верный");
    mockServerTest.mockResolvedValue(steps());
    draw();
    await fillAndNext();
    fireEvent.click(await screen.findByText(ru.ui.wizard.fingerprintOk));
    fireEvent.click(await screen.findByText(ru.ui.wizard.done));
    await screen.findByText(ru.ui.servers.add);
    expect(mockSetActive).not.toHaveBeenCalled();
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
    expect(screen.getByLabelText(ru.ui.wizard.fieldKeyPath)).toHaveValue("/home/u/.ssh/id_ed25519");
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

  it("keeps a profile on the made key when an unrelated field is edited (T626)", async () => {
    // Before T626 the list offered only "By key"/"By password", so a `managed_key` profile
    // opened as "By key" with an empty key path — and whatever a person then saved was not
    // the way this server lets them in any more.
    mockServersList.mockResolvedValue([makeProfile({ auth_kind: "managed_key", key_path: null })]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    expect(await screen.findByLabelText(ru.ui.wizard.fieldAuth)).toHaveValue("managed_key");
    expect(screen.getByText(ru.ui.wizard.authManagedKeyNote)).toBeInTheDocument();
    // Nothing to type for a key nobody types: no secret field that could overwrite it.
    expect(screen.queryByLabelText(ru.ui.wizard.fieldPassword)).toBeNull();
    expect(screen.queryByLabelText(ru.ui.wizard.fieldPassphrase)).toBeNull();

    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldName), {
      target: { value: "Мой сервер 2" },
    });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ name: "Мой сервер 2", auth_kind: "managed_key", key_path: null }),
        null,
      ),
    );
  });

  it("does not offer the made key to a profile that never had one (T626)", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    const select = await screen.findByLabelText(ru.ui.wizard.fieldAuth);
    expect(select.querySelector("option[value='managed_key']")).toBeNull();
  });

  it("shows the core's refusal of a stale way of signing in rather than closing (T626)", async () => {
    mockServersList.mockResolvedValue([makeProfile({ auth_kind: "managed_key", key_path: null })]);
    mockServerUpdate.mockRejectedValue({
      code: "INVALID_INPUT",
      details: [
        { key: "PROFILE_AUTH_NEEDS_SECRET", params: { from: "managed_key", to: "password" } },
      ],
      cause: "auth_kind",
    });
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldAuth), {
      target: { value: "password" },
    });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    expect(
      await screen.findByText(ru.details.PROFILE_AUTH_NEEDS_SECRET, { exact: false }),
    ).toBeInTheDocument();
    // Still on the form: nothing was saved.
    expect(screen.getByText(ru.ui.servers.save)).toBeInTheDocument();
  });

  it("says loudly that the profile may have been left changed when putting it back failed (T644)", async () => {
    mockServersList.mockResolvedValue([makeProfile({ auth_kind: "managed_key", key_path: null })]);
    mockServerUpdate.mockRejectedValue({
      code: "STORAGE_FAILED",
      details: [{ key: "PROFILE_MAY_BE_CHANGED", params: {} }],
      cause:
        "the system's secret store refused the secret (locked), and putting the profile back failed too (the profile changed in the meantime): the profile may have been left changed",
    });
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldAuth), {
      target: { value: "password" },
    });
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldPassword), {
      target: { value: "a-new-password" },
    });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    expect(
      await screen.findByText(ru.details.PROFILE_MAY_BE_CHANGED, { exact: false }),
    ).toBeInTheDocument();
    expect(screen.getByText(/may have been left changed/)).toBeInTheDocument();
    expect(screen.getByText(ru.ui.servers.save)).toBeInTheDocument();
  });

  it("moves off the made key to a key file with no passphrase by sending an empty one (T638)", async () => {
    // The owner's decision 2026-09-30: here, and only here, an empty field is a value — "the
    // file has no passphrase", which also deletes the made key from the store — rather than
    // "keep", which the core refuses for this move.
    mockServersList.mockResolvedValue([makeProfile({ auth_kind: "managed_key", key_path: null })]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldAuth), {
      target: { value: "key" },
    });
    // The field under it says what happens now, not "leave empty to keep".
    expect(screen.getByText(ru.ui.servers.leaveMadeKeyForFileHint)).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.servers.editSecretHint)).toBeNull();
    expect(screen.getByLabelText(ru.ui.wizard.fieldPassphrase)).toHaveValue("");
    fireEvent.change(screen.getByLabelText(ru.ui.wizard.fieldKeyPath), {
      target: { value: "C:/keys/id_ed25519" },
    });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ auth_kind: "key", key_path: "C:/keys/id_ed25519" }),
        "",
      ),
    );
  });

  it("moving off the made key to a password still sends no empty password (T638)", async () => {
    // An empty password is no password: the form sends `null`, and the core refuses the move
    // (`PROFILE_AUTH_NEEDS_SECRET`) — the guard against a stale form stays as it was.
    mockServersList.mockResolvedValue([makeProfile({ auth_kind: "managed_key", key_path: null })]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    fireEvent.change(await screen.findByLabelText(ru.ui.wizard.fieldAuth), {
      target: { value: "password" },
    });
    expect(screen.getByText(ru.ui.servers.leaveMadeKeyForPasswordHint)).toBeInTheDocument();
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ auth_kind: "password" }),
        null,
      ),
    );
  });

  it("an empty passphrase on an ordinary key profile still means keep (T638)", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    expect(await screen.findByText(ru.ui.servers.editSecretHint)).toBeInTheDocument();
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ auth_kind: "key" }),
        null,
      ),
    );
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

  it("says why a server is somebody else's in words, never as JSON (T709)", async () => {
    mockServerDetect.mockResolvedValue(
      managedState({
        kind: "Foreign",
        server_version: null,
        foreign_reason: { StateFileUnreadable: { problem: "NoVersion" } },
      }),
    );
    const view = renderIn(<ServerStateCard serverId="srv_1" />, "ru");

    expect(await screen.findByText(ru.ui.serverState.foreignStateBroken)).toBeInTheDocument();
    expect(view.container.textContent).not.toMatch(/[{}]|StateFileUnreadable|NoVersion/);
  });

  it("names the web server that is already running (T709)", async () => {
    mockServerDetect.mockResolvedValue(
      managedState({
        kind: "Foreign",
        server_version: null,
        foreign_reason: { WebServerRunning: { name: "nginx" } },
      }),
    );
    renderIn(<ServerStateCard serverId="srv_1" />, "en");
    expect(
      await screen.findByText(en.ui.serverState.foreignWebServer("nginx")),
    ).toBeInTheDocument();
  });
});

describe("a section with no server (T709)", () => {
  it("says to add a server first when there are none, and to choose one when there are", async () => {
    const { NoServer } = await import("../../shared/NoServer");
    useServers.setState({ profiles: [], loading: false, error: null });
    const view = renderIn(
      <MemoryRouter>
        <NoServer />
      </MemoryRouter>,
      "ru",
    );
    expect(screen.getByText(ru.ui.common.noServers, { exact: false })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: ru.ui.common.toServers })).toBeInTheDocument();
    view.unmount();

    useServers.setState({
      profiles: [makeProfile({ is_active: false }), makeProfile({ id: "srv_2", is_active: false })],
      loading: false,
      error: null,
    });
    renderIn(
      <MemoryRouter>
        <NoServer />
      </MemoryRouter>,
      "ru",
    );
    expect(screen.getByText(ru.ui.common.noActiveServer, { exact: false })).toBeInTheDocument();
  });
});

/** T712 — the hosting plan on the server's card. */
describe("T712 — «Тариф, Мбит/с» on the server's card", () => {
  it("is shown filled from the profile and sent with the rest", async () => {
    mockServersList.mockResolvedValue([makeProfile({ tariff_mbit: 300 })]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    const field = await screen.findByLabelText(ru.ui.wizard.fieldTariff);
    expect(field).toHaveValue(300);
    fireEvent.change(field, { target: { value: "500" } });
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ tariff_mbit: 500 }),
        null,
      ),
    );
  });

  it("left empty, says the network card is what counts, and sends none", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerUpdate.mockResolvedValue(undefined);
    draw();

    fireEvent.click(await screen.findByText(ru.ui.servers.edit));
    const field = await screen.findByLabelText(ru.ui.wizard.fieldTariff);
    expect(field).toHaveValue(null);
    expect(field).toHaveAttribute("placeholder", "по сетевой карте");
    fireEvent.click(screen.getByText(ru.ui.servers.save));

    await waitFor(() =>
      expect(mockServerUpdate).toHaveBeenCalledWith(
        "srv_1",
        expect.objectContaining({ tariff_mbit: null }),
        null,
      ),
    );
  });

  it("is short and said in both languages", () => {
    for (const label of [ru.ui.wizard.fieldTariff, en.ui.wizard.fieldTariff]) {
      expect(label.length).toBeLessThanOrEqual(90);
    }
    expect(en.ui.wizard.fieldTariff).toBe("Plan, Mbit/s");
  });
});
