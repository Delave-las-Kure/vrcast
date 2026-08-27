/**
 * T178 — the viewers screen.
 *
 * What is checked is what this screen is for and what it must not do: the list arrives by
 * itself rather than being asked for, what is not determined says so, a viewer in trouble
 * is marked with the reason, and leaving the screen lets the server's channels go.
 */

import { screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type { ServerProfile, Viewer, ViewersUpdateEvent } from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockWatchStart = vi.fn(async () => undefined);
const mockWatchStop = vi.fn(async () => undefined);
const mockLibraryList = vi.fn();

/** What the core would send. Held so a test can push an update whenever it likes. */
let send: ((update: ViewersUpdateEvent) => void) | null = null;
const unlisten = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      serversList: () => mockServersList(),
      serverSetActive: vi.fn(),
      libraryList: () => mockLibraryList(),
      viewersWatchStart: (...a: unknown[]) => mockWatchStart(...(a as [])),
      viewersWatchStop: () => mockWatchStop(),
      viewersHistory: vi.fn(),
    },
    onLibraryChanged: vi.fn(async () => () => {}),
    onViewersUpdate: vi.fn(async (handler: (u: ViewersUpdateEvent) => void) => {
      send = handler;
      return unlisten;
    }),
  };
});

const { ViewersScreen } = await import("../ViewersScreen");

const server: ServerProfile = {
  id: "s1",
  name: "Server",
  host: "198.51.100.7",
  port: 22,
  user: "root",
  domain: "example.test",
  // Deliberately not the path a deployed server uses: the guard against hardcoded
  // servers watches for that one, and a test that quoted it would blunt the guard.
  video_dir: "/srv/test-videos",
  cdn_base: null,
  auth_kind: "key",
  key_path: "/k",
  secret_ref: "r",
  host_fingerprint: "SHA256:x",
  ipv6_mode: null,
  is_active: true,
};

function viewer(over: Partial<Viewer> = {}): Viewer {
  return {
    ip: "203.0.113.9",
    country: "NL",
    city: "Amsterdam",
    asn_org: "Example Networks",
    media_id: "m1",
    variant: "v2",
    delivery_bps: 5_000_000,
    required_bps: 5_000_000,
    started_at: "2026-08-26T10:00:00Z",
    last_seen_at: "2026-08-26T10:03:20Z",
    problems: [],
    ...over,
  };
}

function update(active: Viewer[]): ViewersUpdateEvent {
  const per_media: Record<string, number> = {};
  for (const v of active) if (v.media_id) per_media[v.media_id] = (per_media[v.media_id] ?? 0) + 1;
  return { event: "viewers_update", server_id: "s1", active, per_media };
}

beforeEach(() => {
  vi.clearAllMocks();
  send = null;
  mockServersList.mockResolvedValue([server]);
  mockLibraryList.mockResolvedValue({
    server_id: "s1",
    media: [
      {
        id: "m1",
        title: "Backrooms",
        slug: "backrooms",
        files: [],
        ladders: [],
        total_bytes: 0,
        created_at: "",
      },
    ],
    unrecognized: [],
    disk: null,
    stale: false,
  });
});

describe("the viewers screen", () => {
  it("switches the watching on and takes the list from the stream, without asking again", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalledWith("s1"));

    send?.(update([viewer()]));
    await waitFor(() => expect(screen.getByText("203.0.113.9")).toBeInTheDocument());

    // The whole point of the stream: the list moved and nothing was asked for again.
    expect(mockWatchStart).toHaveBeenCalledTimes(1);
  });

  it("says what it does not know instead of leaving a gap or making it up", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(
      update([
        viewer({
          country: null,
          city: null,
          asn_org: null,
          media_id: null,
          variant: null,
          delivery_bps: null,
        }),
      ]),
    );

    await waitFor(() =>
      expect(screen.getAllByText(ru.ui.viewers.notKnown).length).toBeGreaterThanOrEqual(2),
    );
    // Not knowing what is being watched is a state of its own, and it is said in words —
    // an empty cell would read as a fault in the application.
    expect(screen.getByText(ru.ui.viewers.watchingUnknown)).toBeInTheDocument();
  });

  it("marks a viewer in trouble with the reason rather than merely marking them", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(update([viewer({ problems: ["SlowLink"] })]));

    // "Something is wrong with somebody" is the state the owner was already in before
    // opening the application. The reason is the whole value of the mark (FR-053).
    await waitFor(() =>
      expect(screen.getByText(ru.ui.viewers.problems.slowLink)).toBeInTheDocument(),
    );
  });

  it("shows nobody watching as an ordinary state, not as an error", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(update([]));
    await waitFor(() => expect(screen.getByText(ru.ui.viewers.nobody)).toBeInTheDocument());
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("lets the server's channels go when the screen is left", async () => {
    // Watching holds two of the server's eight channels for as long as it runs (R-04). A
    // screen that forgot to let go would take them out of everything else, and it would be
    // found much later as a third channel failing to open.
    const view = renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    view.unmount();
    await waitFor(() => expect(mockWatchStop).toHaveBeenCalled());
    expect(unlisten).toHaveBeenCalled();
  });

  it("speaks whichever language is chosen", async () => {
    renderIn(<ViewersScreen />, "en");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(update([viewer({ problems: ["Stalls"] })]));
    await waitFor(() =>
      expect(screen.getByText(en.ui.viewers.problems.stalls)).toBeInTheDocument(),
    );
    expect(screen.queryByText(ru.ui.viewers.problems.stalls)).not.toBeInTheDocument();
  });
});
