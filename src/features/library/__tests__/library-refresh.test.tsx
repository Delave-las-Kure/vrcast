/**
 * T651 + T657 — how the library screen reads the library again.
 *
 * QA-24A №2: every `library:changed` answered with `libraryList(id, false)`, which asked the
 * core for another background refresh, whose end sent another `library:changed` — the open
 * library reread the whole server for as long as it stayed open. The event is the end of a
 * refresh, so the screen now reads what is known (`libraryKnown`) and asks for nothing.
 *
 * QA-24A №8: answers were shown in the order they arrived, not the order they were asked
 * for, so an old answer arriving last put the old title back over the new one. Only the
 * latest request's answer — data, error and end of loading alike — is shown now.
 *
 * Built on the QA round's own probes (`qa24a/ui.test.tsx`), turned round to assert the
 * wanted behaviour.
 */

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderIn, ru } from "../../../test-utils";
import type { AppError, LibraryView, MediaView, ServerProfile } from "../../../shared/contract";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  known: vi.fn(),
  changed: new Set<(serverId: string) => void>(),
}));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      // Whatever the test has put into the store: a reload must not undo it.
      serversList: async () => useServers.getState().profiles,
      libraryList: (...a: unknown[]) => mocks.list(...a),
      libraryKnown: (...a: unknown[]) => mocks.known(...a),
      librarySuggestGroups: async () => ({ groups: [], singles: [] }),
    }),
    onLibraryChanged: async (f: (serverId: string) => void) => {
      mocks.changed.add(f);
      return () => mocks.changed.delete(f);
    },
    onViewersUpdate: vi.fn(async () => () => {}),
  };
});

const { LibraryScreen } = await import("../LibraryScreen");
const { useServers } = await import("../../servers/store");

function profile(): ServerProfile {
  return {
    id: "s1",
    name: "QA server",
    host: "localhost",
    port: 22,
    user: "qa",
    auth_kind: "key",
    secret_ref: "unused",
    key_path: null,
    domain: "test.invalid",
    video_dir: "/qa/videos",
    cdn_base: null,
    host_fingerprint: "SHA256:test",
    ipv6_mode: null,
    is_active: true,
  };
}

const medium = (title: string): MediaView => ({
  id: "m1",
  title,
  slug: "one",
  files: [],
  ladders: [],
  total_bytes: 0,
  created_at: "2026-01-01T00:00:00Z",
});

const view = (title = "Current title"): LibraryView => ({
  server_id: "s1",
  media: [medium(title)],
  unrecognized: [],
  disk: null,
  stale: false,
});

function deferred<T>() {
  let resolve!: (x: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const draw = () =>
  renderIn(
    <MemoryRouter>
      <LibraryScreen />
    </MemoryRouter>,
  );

const fire = (serverId = "s1") =>
  act(async () => {
    mocks.changed.forEach((f) => f(serverId));
  });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.list.mockReset();
  mocks.known.mockReset();
  mocks.changed.clear();
  useServers.setState({ profiles: [profile()], loading: false, error: null });
  mocks.list.mockResolvedValue(view());
  mocks.known.mockResolvedValue(view());
});

describe("T651 — library:changed does not ask for another refresh", () => {
  it("reads what is known on each event and never asks the server again", async () => {
    draw();
    await screen.findByText("Current title");
    await waitFor(() => expect(mocks.changed.size).toBe(1));

    for (let i = 0; i < 3; i++) await fire();

    // One opening read — the one that lets a refresh follow the cache — and nothing more.
    expect(mocks.list.mock.calls).toEqual([["s1", false]]);
    expect(mocks.known.mock.calls).toEqual([["s1"], ["s1"], ["s1"]]);
  });

  it("shows what the event brings", async () => {
    draw();
    await screen.findByText("Current title");
    await waitFor(() => expect(mocks.changed.size).toBe(1));
    mocks.known.mockResolvedValueOnce(view("Renamed elsewhere"));

    await fire();

    expect(await screen.findByText("Renamed elsewhere")).toBeInTheDocument();
  });

  it("ignores another server's change", async () => {
    draw();
    await screen.findByText("Current title");
    await waitFor(() => expect(mocks.changed.size).toBe(1));

    await fire("s2");

    expect(mocks.known).not.toHaveBeenCalled();
    expect(mocks.list).toHaveBeenCalledTimes(1);
  });
});

describe("T657 — only the latest answer is shown", () => {
  it("an old answer arriving last does not overwrite the newer one", async () => {
    draw();
    await screen.findByText("Current title");
    const old = deferred<LibraryView>();
    const fresh = deferred<LibraryView>();
    mocks.list.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const button = screen.getByRole("button", { name: ru.ui.common.refresh });
    fireEvent.click(button);
    fireEvent.click(button);

    await act(async () => fresh.resolve(view("New confirmed title")));
    expect(screen.getByText("New confirmed title")).toBeInTheDocument();

    await act(async () => old.resolve(view("Old title")));
    expect(screen.getByText("New confirmed title")).toBeInTheDocument();
    expect(screen.queryByText("Old title")).not.toBeInTheDocument();
  });

  it("an old failure arriving last does not put an error over the newer answer", async () => {
    draw();
    await screen.findByText("Current title");
    const old = deferred<LibraryView>();
    const fresh = deferred<LibraryView>();
    mocks.list.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const button = screen.getByRole("button", { name: ru.ui.common.refresh });
    fireEvent.click(button);
    fireEvent.click(button);

    await act(async () => fresh.resolve(view("New confirmed title")));
    const failure: AppError = { code: "SSH_UNREACHABLE" } as AppError;
    await act(async () => old.reject(failure));

    expect(screen.getByText("New confirmed title")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("an event's answer overtaken by a refresh is not shown", async () => {
    draw();
    await screen.findByText("Current title");
    await waitFor(() => expect(mocks.changed.size).toBe(1));
    const eventAnswer = deferred<LibraryView>();
    mocks.known.mockReturnValueOnce(eventAnswer.promise);
    mocks.list.mockResolvedValueOnce(view("After refresh"));

    await fire();
    fireEvent.click(screen.getByRole("button", { name: ru.ui.common.refresh }));
    expect(await screen.findByText("After refresh")).toBeInTheDocument();

    await act(async () => eventAnswer.resolve(view("Before refresh")));
    expect(screen.getByText("After refresh")).toBeInTheDocument();
    expect(screen.queryByText("Before refresh")).not.toBeInTheDocument();
  });

  it("the first read's late end does not end the loading of a later one", async () => {
    // Loading belongs to the latest request too: the screen stays on "reading" until the
    // answer it is actually waiting for has come.
    const first = deferred<LibraryView>();
    mocks.list.mockReturnValueOnce(first.promise);
    draw();
    // The screen's own reload of the servers settles first; it must not undo the switch.
    await act(async () => {});
    expect(screen.getByText(ru.ui.library.reading)).toBeInTheDocument();

    // Another active server makes the screen read again, from the start.
    const second = deferred<LibraryView>();
    mocks.list.mockReturnValueOnce(second.promise);
    await act(async () => {
      useServers.setState({ profiles: [{ ...profile(), id: "s2", name: "Other server" }] });
    });
    expect(mocks.list.mock.calls).toEqual([
      ["s1", false],
      ["s2", false],
    ]);

    await act(async () => first.resolve(view("First server's answer")));
    expect(screen.queryByText("First server's answer")).not.toBeInTheDocument();
    expect(screen.getByText(ru.ui.library.reading)).toBeInTheDocument();

    await act(async () => second.resolve(view("Second answer")));
    expect(await screen.findByText("Second answer")).toBeInTheDocument();
  });
});
