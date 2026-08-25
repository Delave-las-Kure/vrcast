/**
 * T059 — тесты раздела серверов.
 *
 * Ядро подменено: тест интерфейса не должен требовать ни сервера, ни базы.
 * Проверяется то, на чём человек обожжётся в жизни: что подтверждение отпечатка
 * нельзя пропустить, что видны все шаги проверки, а не только сломавшийся, и что
 * профиль без подтверждённого отпечатка не выглядит рабочим.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ServerProfile, TestStep } from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockServerAdd = vi.fn();
const mockServerTest = vi.fn<() => Promise<TestStep[]>>();
const mockProbeFingerprint = vi.fn<() => Promise<string>>();
const mockConfirmFingerprint = vi.fn();
const mockServerRemove = vi.fn();
const mockSetActive = vi.fn();
const mockImportSuggestion = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual =
    await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
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
    video_dir: "/var/lib/vrcast/videos",
    cdn_base: null,
    host_fingerprint: "SHA256:тестовыйОтпечаток",
    ipv6_mode: null,
    is_active: true,
    ...over,
  };
}

function steps(): TestStep[] {
  return [
    { id: "network", title: "Сервер доступен по сети", status: "failed", detail: "порт закрыт" },
    { id: "login", title: "Вход на сервер", status: "skipped", detail: null },
    { id: "video_dir", title: "Каталог с видео доступен", status: "skipped", detail: null },
    { id: "domain", title: "Раздача отвечает по домену", status: "skipped", detail: null },
  ];
}

const draw = () =>
  render(
    <MemoryRouter>
      <ServerList />
    </MemoryRouter>,
  );

beforeEach(() => {
  vi.clearAllMocks();
  // Хранилище состояния общее на весь модуль и переживает размонтирование:
  // без сброса следующий тест увидит профили предыдущего.
  useServers.setState({ profiles: [], loading: true, error: null });
  mockServersList.mockResolvedValue([]);
  mockImportSuggestion.mockResolvedValue(null);
});

describe("список серверов", () => {
  it("объясняет пустоту, а не показывает пустой экран", async () => {
    draw();
    expect(await screen.findByText(/Серверов пока нет/)).toBeInTheDocument();
  });

  it("показывает адрес и домен сервера", async () => {
    mockServersList.mockResolvedValue([makeProfile()]);
    draw();

    expect(await screen.findByText("Мой сервер")).toBeInTheDocument();
    expect(screen.getByText("root@203.0.113.10")).toBeInTheDocument();
    expect(screen.getByText("stream.example.com")).toBeInTheDocument();
  });

  it("помечает профиль без подтверждённого отпечатка", async () => {
    // Такой профиль существует, но подключиться по нему нельзя. Молчать об этом —
    // значит оставить человека гадать, почему ничего не работает.
    mockServersList.mockResolvedValue([makeProfile({ host_fingerprint: null })]);
    draw();

    expect(
      await screen.findByText(/Отпечаток сервера не подтверждён/),
    ).toBeInTheDocument();
  });

  it("не спрашивает про удаление вслепую", async () => {
    // FR-005: вместе с профилем забывается и доступ к серверу. Об этом надо сказать
    // до нажатия, а не после.
    mockServersList.mockResolvedValue([makeProfile()]);
    draw();

    fireEvent.click(await screen.findByText("Удалить"));
    expect(await screen.findByText(/Пароль или ключ.*тоже будут забыты/)).toBeInTheDocument();
    expect(mockServerRemove).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Да, удалить"));
    await waitFor(() => expect(mockServerRemove).toHaveBeenCalledWith("srv_1"));
  });

  it("показывает все шаги проверки, включая невыполнявшиеся", async () => {
    // FR-003. Человеку нужно видеть, что успело пройти, а не только последнюю беду.
    mockServersList.mockResolvedValue([makeProfile()]);
    mockServerTest.mockResolvedValue(steps());
    draw();

    fireEvent.click(await screen.findByText("Проверить подключение"));

    expect(await screen.findByText("Сервер доступен по сети")).toBeInTheDocument();
    expect(screen.getByText("порт закрыт")).toBeInTheDocument();
    // Шаги после сломавшегося тоже на экране — с пояснением, почему их не смотрели.
    expect(screen.getByText("Вход на сервер")).toBeInTheDocument();
    expect(screen.getByText("Раздача отвечает по домену")).toBeInTheDocument();
    expect(screen.getAllByText(/остановились раньше/).length).toBe(3);
  });
});

describe("мастер настройки", () => {
  it("требует подтвердить отпечаток до проверки подключения", async () => {
    // Это единственный шаг, который нельзя пропустить: до подтверждения приложение
    // не отправляет серверу ни пароль, ни ключ (FR-092).
    mockServerAdd.mockResolvedValue("srv_new");
    mockProbeFingerprint.mockResolvedValue("SHA256:ОтпечатокНовогоСервера");
    mockServerTest.mockResolvedValue(steps());
    draw();

    fireEvent.click(await screen.findByText("Добавить сервер"));

    fireEvent.change(screen.getByLabelText("Название"), { target: { value: "Тест" } });
    fireEvent.change(screen.getByLabelText("Адрес"), { target: { value: "203.0.113.10" } });
    fireEvent.change(screen.getByLabelText("Домен раздачи"), {
      target: { value: "stream.example.com" },
    });
    fireEvent.change(screen.getByLabelText("Путь к приватному ключу"), {
      target: { value: "/home/u/.ssh/k" },
    });
    fireEvent.click(screen.getByText("Дальше"));

    // Отпечаток показан, проверка ещё не запускалась.
    expect(await screen.findByText("SHA256:ОтпечатокНовогоСервера")).toBeInTheDocument();
    expect(mockServerTest).not.toHaveBeenCalled();
    expect(mockConfirmFingerprint).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Отпечаток верный"));
    await waitFor(() =>
      expect(mockConfirmFingerprint).toHaveBeenCalledWith(
        "srv_new",
        "SHA256:ОтпечатокНовогоСервера",
      ),
    );
    await waitFor(() => expect(mockServerTest).toHaveBeenCalled());
  });

  it("убирает за собой профиль, если человек отказался на шаге отпечатка", async () => {
    // Иначе в списке останется полусозданный сервер, по которому нельзя подключиться,
    // и человек не поймёт, откуда он взялся.
    mockServerAdd.mockResolvedValue("srv_new");
    mockProbeFingerprint.mockResolvedValue("SHA256:чужой");
    draw();

    fireEvent.click(await screen.findByText("Добавить сервер"));
    fireEvent.change(screen.getByLabelText("Название"), { target: { value: "Тест" } });
    fireEvent.change(screen.getByLabelText("Адрес"), { target: { value: "203.0.113.10" } });
    fireEvent.change(screen.getByLabelText("Домен раздачи"), {
      target: { value: "stream.example.com" },
    });
    fireEvent.change(screen.getByLabelText("Путь к приватному ключу"), {
      target: { value: "/home/u/.ssh/k" },
    });
    fireEvent.click(screen.getByText("Дальше"));

    fireEvent.click(await screen.findByText("Отказаться"));
    await waitFor(() => expect(mockServerRemove).toHaveBeenCalledWith("srv_new"));
  });

  it("предлагает перенести настройки, когда рядом нашёлся прежний файл", async () => {
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

    fireEvent.click(await screen.findByText("Добавить сервер"));
    expect(await screen.findByText(/Рядом нашлись настройки/)).toBeInTheDocument();
    // Про парольную фразу сказано честно: в файле её нет и не может быть.
    expect(screen.getByText(/Парольную фразу ключа придётся ввести/)).toBeInTheDocument();

    fireEvent.click(screen.getByText("Подставить"));
    await waitFor(() =>
      expect(screen.getByLabelText("Адрес")).toHaveValue("203.0.113.10"),
    );
  });
});
