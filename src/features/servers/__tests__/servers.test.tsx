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
import type { ServerProfile, TestStep } from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockServerAdd = vi.fn();
const mockServerTest = vi.fn<() => Promise<TestStep[]>>();
const mockProbeFingerprint = vi.fn<() => Promise<string>>();
const mockConfirmFingerprint = vi.fn();
const mockServerRemove = vi.fn();
const mockSetActive = vi.fn();
const mockServerDetect = vi.fn<() => Promise<never>>(() =>
  Promise.reject({ code: "SSH_UNREACHABLE" }),
);
const mockImportSuggestion = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      serversList: () => mockServersList(),
      serverAdd: (...a: unknown[]) => mockServerAdd(...a),
      serverUpdate: vi.fn(),
      serverRemove: (...a: unknown[]) => mockServerRemove(...a),
      serverSetActive: (...a: unknown[]) => mockSetActive(...a),
      serverTest: (...a: unknown[]) => mockServerTest(...(a as [])),
      serverFingerprintConfirm: (...a: unknown[]) => mockConfirmFingerprint(...a),
      serverProbeFingerprint: (...a: unknown[]) => mockProbeFingerprint(...(a as [])),
      serverImportSuggestion: () => mockImportSuggestion(),
      // Карточка сервера с T294 спрашивает, что это за сервер. Здесь он не отвечает — и
      // это состояние настоящее: сервер, который молчит, не должен ронять список.
      serverDetect: () => mockServerDetect(),
    },
  };
});

const { ServerList } = await import("../ServerList");
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
