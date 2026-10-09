/**
 * T178 — the viewers screen.
 *
 * What is checked is what this screen is for and what it must not do: the list arrives by
 * itself rather than being asked for, what is not determined says so, a viewer in trouble
 * is marked with the reason, and leaving the screen lets the server's channels go.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type {
  GeoStatus,
  LadderSetView,
  ServerProfile,
  Viewer,
  ViewersUpdateEvent,
} from "../../../shared/contract";

const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockWatchStart = vi.fn(async () => undefined);
const mockWatchStop = vi.fn(async () => undefined);
const mockLibraryList = vi.fn();
const mockLimitPreview = vi.fn(async (_request: { slug: string }) => ({
  kept: [
    {
      path: "backrooms_6.mp4",
      bandwidth: 6_000_000,
      average_bandwidth: 6_000_000,
      width: 1920,
      height: 1080,
      fps: 24,
      codecs: "avc1",
    },
  ],
  warnings: [],
  below_lightest: false,
}));
const mockLimitSet = vi.fn(async (..._a: unknown[]) => undefined);

/** What the core would send. Held so a test can push an update whenever it likes. */
let send: ((update: ViewersUpdateEvent) => void) | null = null;
const unlisten = vi.fn();

/** The tables of places. Ready and current by default: that is the ordinary state, and the
 *  screen must then say nothing about them at all. */
const mockGeoStatus = vi.fn<() => Promise<GeoStatus>>(() =>
  Promise.resolve({ month: "2026-08", ready: true, stale: false }),
);
const mockGeoUpdate = vi.fn<() => Promise<GeoStatus>>(() =>
  Promise.resolve({ month: "2026-08", ready: true, stale: false }),
);

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      serversList: () => mockServersList(),
      serverSetActive: vi.fn(),
      libraryList: () => mockLibraryList(),
      viewersWatchStart: (...a: unknown[]) => mockWatchStart(...(a as [])),
      viewersWatchStop: () => mockWatchStop(),
      viewersHistory: vi.fn(),
      geoStatus: () => mockGeoStatus(),
      geoUpdate: () => mockGeoUpdate(),
      limitPreview: (...a: unknown[]) => mockLimitPreview(...(a as [{ slug: string }])),
      limitSet: (...a: unknown[]) => mockLimitSet(...a),
      limitsList: () => Promise.resolve([]),
    }),
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

/** A quality set on the server, as the library reports one (T704: only these can be capped). */
function aSet(slug: string): LadderSetView {
  return {
    path: `${slug}/master.m3u8`,
    size_bytes: 1,
    width: 1920,
    height: 1080,
    bitrate_bps: 6_000_000,
    duration_s: 60,
    exists_on_server: true,
    origin_url: `https://stream.example.com/videos/${slug}/master.m3u8`,
    cdn_url: null,
  };
}

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

function update(
  active: Viewer[],
  watch: Partial<Pick<ViewersUpdateEvent, "watch" | "as_of" | "attempt">> = {},
): ViewersUpdateEvent {
  const per_media: Record<string, number> = {};
  for (const v of active) if (v.media_id) per_media[v.media_id] = (per_media[v.media_id] ?? 0) + 1;
  return {
    event: "viewers_update",
    server_id: "s1",
    active,
    per_media,
    watch: "watching",
    as_of: new Date().toISOString(),
    attempt: 0,
    ...watch,
  };
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
        ladders: [aSet("backrooms")],
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

  it("says «в порядке» only when both speeds are known, and «данных пока нет» otherwise (T705)", async () => {
    // QA-26 №12: a viewer whose speed, need and film were all unknown was shown as fine.
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());
    send?.(
      update([
        viewer({ ip: "203.0.113.1", delivery_bps: null }),
        viewer({ ip: "203.0.113.2", required_bps: null }),
        viewer({ ip: "203.0.113.3" }),
        viewer({ ip: "203.0.113.4", delivery_bps: 1_000_000, problems: ["SlowLink"] }),
      ]),
    );
    await waitFor(() => expect(screen.getAllByTestId("viewer-no-data")).toHaveLength(2));
    expect(screen.getAllByText(ru.ui.viewers.noData)).toHaveLength(2);
    expect(screen.getAllByText(ru.ui.viewers.fine)).toHaveLength(1);
    expect(screen.getByText(ru.ui.viewers.problems.slowLink)).toBeInTheDocument();
    expect(ru.ui.viewers.problems.slowLink).toBe("не успевает");
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

/**
 * T664 — QA-24B-05. After a break in the connection the screen used to go on showing the last
 * list as if it were now, with nothing to say it was old. The core now says where the
 * watching stands on every update; the screen must say it too.
 */
describe("when the connection to the server is lost", () => {
  it("says it is reconnecting, keeps the last list marked as old, and says how old", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(update([viewer()]));
    const table = await screen.findByTestId("viewers-table");
    expect(table.getAttribute("data-stale")).toBe("false");
    expect(screen.queryByTestId("viewers-reconnecting")).toBeNull();

    const fortySecondsAgo = new Date(Date.now() - 40_000).toISOString();
    send?.(update([viewer()], { watch: "reconnecting", as_of: fortySecondsAgo, attempt: 2 }));

    const notice = await screen.findByTestId("viewers-reconnecting");
    expect(notice.textContent).toContain(ru.ui.viewers.reconnecting);
    expect(notice.textContent).toContain("2");
    // The age, in words, not only a colour.
    expect(screen.getByTestId("viewers-age").textContent).toMatch(/4\d с/);
    // The list is still there — it is the best there is — but it is marked.
    expect(screen.getByText("203.0.113.9")).toBeInTheDocument();
    expect(screen.getByTestId("viewers-table").getAttribute("data-stale")).toBe("true");
  });

  it("does not call an old empty list 'nobody is watching'", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(update([], { watch: "reconnecting", attempt: 1 }));
    await screen.findByTestId("viewers-reconnecting");
    expect(screen.queryByText(ru.ui.viewers.nobody)).toBeNull();
  });

  it("drops the marks as soon as a current list arrives again", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalled());

    send?.(update([viewer()], { watch: "reconnecting", attempt: 3 }));
    await screen.findByTestId("viewers-reconnecting");

    send?.(update([viewer()]));
    await waitFor(() => expect(screen.queryByTestId("viewers-reconnecting")).toBeNull());
    expect(screen.getByTestId("viewers-table").getAttribute("data-stale")).toBe("false");
  });

  it("when it has given up, says so and offers to start again — which starts it again", async () => {
    renderIn(<ViewersScreen />, "en");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalledTimes(1));

    send?.(update([viewer()], { watch: "stopped", attempt: 4 }));
    const stopped = await screen.findByTestId("viewers-stopped");
    expect(stopped.textContent).toContain(en.ui.viewers.stopped);

    fireEvent.click(screen.getByRole("button", { name: en.ui.viewers.restart }));
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalledTimes(2));
  });
});

/**
 * T707 (tour I04–I05) — a server that could not be reached. It used to be one line, "could not
 * reach the server", a minute later as much as at once; closing it left "starting…" for good.
 */
describe("when the server cannot be reached at all", () => {
  const unreachable = { code: "SSH_UNREACHABLE", details: [] };

  it("offers to try again, and trying again starts the watching again", async () => {
    mockWatchStart.mockRejectedValueOnce(unreachable);
    renderIn(<ViewersScreen />, "ru");
    const retry = await screen.findByRole("button", { name: ru.ui.viewers.retry });
    expect(screen.getByRole("alert")).toBeInTheDocument();
    // The notice cannot be closed into an empty "starting…".
    expect(screen.queryByLabelText(ru.ui.common.dismiss)).toBeNull();

    fireEvent.click(retry);
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalledTimes(2));
    send?.(update([]));
    await screen.findByText(ru.ui.viewers.nobody);
    expect(screen.queryByTestId("viewers-retry")).toBeNull();
  });

  it("tries again by itself, waiting longer each time, and says when", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      mockWatchStart.mockRejectedValueOnce(unreachable).mockRejectedValueOnce(unreachable);
      renderIn(<ViewersScreen />, "ru");
      await screen.findByTestId("viewers-retry-in");
      expect(screen.getByTestId("viewers-retry-in").textContent).toMatch(/через [45] с/);
      expect(mockWatchStart).toHaveBeenCalledTimes(1);

      await vi.advanceTimersByTimeAsync(5_000);
      await waitFor(() => expect(mockWatchStart).toHaveBeenCalledTimes(2));
      // The second failure in a row waits longer.
      await waitFor(() =>
        expect(screen.getByTestId("viewers-retry-in").textContent).toMatch(/через (9|10) с/),
      );

      await vi.advanceTimersByTimeAsync(10_000);
      await waitFor(() => expect(mockWatchStart).toHaveBeenCalledTimes(3));
      // The third try succeeded: the screen is back to its ordinary self.
      send?.(update([]));
      await screen.findByText(ru.ui.viewers.nobody);
      expect(screen.queryByRole("alert")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not try a refused sign-in again by itself — only when asked", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      mockWatchStart.mockRejectedValueOnce({ code: "SSH_AUTH_FAILED", details: [] });
      renderIn(<ViewersScreen />, "en");
      await screen.findByRole("button", { name: en.ui.viewers.retry });
      expect(screen.queryByTestId("viewers-retry-in")).toBeNull();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(mockWatchStart).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("the tables of places", () => {
  it("says nothing while they are there and current", async () => {
    // The ordinary state. A line reporting it on every visit is noise, and noise in a corner
    // of the screen teaches people to skip that corner — including the times it matters.
    renderIn(<ViewersScreen />);
    await waitFor(() => expect(screen.getByText(ru.ui.sections.viewers)).toBeTruthy());
    expect(screen.queryByTestId("places-tables")).toBeNull();
  });

  it("offers to fetch them when there are none, because otherwise no place is shown at all", async () => {
    // The gap this closes: the commands existed, were registered and described, and nothing
    // called them. With the tables missing, every viewer showed no country and there was
    // nothing on screen to press.
    mockGeoStatus.mockResolvedValueOnce({ month: null, ready: false, stale: true });
    renderIn(<ViewersScreen />);

    const line = await screen.findByTestId("places-tables");
    expect(line.textContent).toContain(ru.ui.viewers.placesMissing);

    fireEvent.click(screen.getByText(ru.ui.viewers.placesFetch));
    await waitFor(() => expect(mockGeoUpdate).toHaveBeenCalled());
  });

  /**
   * ⚠ **T501 — the cap dialog was handed identifiers where the core wanted slugs.**
   *
   * `useMediaTitles` returned `{ [media.id]: title }` and the screen read those entries as
   * `([slug, title]) => ({ slug, title })`: an identifier renamed to a slug, nothing more.
   * Identifiers are `m_<uuid>`; the core builds `{video_dir}/{slug}/master.m3u8`, so the
   * answer was `NoLadderForMedia` every time, the preview never filled and the confirm
   * button stayed disabled. FR-060 and FR-061 could not be reached from this screen at all.
   *
   * **Why nothing caught it.** `limits.test.tsx` renders `LimitDialog` on its own and hands
   * it `[{ slug: "demo", title: "Demo film" }]` — written by hand, correct, and therefore
   * silent about where the screen gets that value. The stand and the real screen diverged at
   * exactly the point of the defect. So this one goes through the screen, with a medium whose
   * identifier and slug differ, which is the ordinary case and the only one that can tell
   * them apart.
   */
  it("asks the core about the medium by its slug, not by its identifier", async () => {
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalledWith("s1"));
    send?.(update([viewer()]));

    // The row names the film first, and by the identifier the viewer record carries — the
    // other half of the same confusion, and nothing checked it either: breaking this on
    // purpose passed every test there was.
    expect(await screen.findByText("Backrooms")).toBeInTheDocument();

    fireEvent.click(await screen.findByRole("button", { name: ru.ui.limits.title }));

    await waitFor(() => expect(mockLimitPreview).toHaveBeenCalled());
    const asked = mockLimitPreview.mock.calls[0][0];
    expect(
      asked.slug,
      "the dialog asked the core about the medium by something that is not its slug, so the " +
        "core looks for a quality set at a path that does not exist",
    ).toBe("backrooms");
  });

  it("opens the cap on the film the viewer is watching, not the first in the catalogue (T668)", async () => {
    // QA-24B-09: a viewer of film B got a dialog set to film A, and putting it right was a
    // fourth action where SC-006 allows three.
    mockLibraryList.mockResolvedValue({
      server_id: "s1",
      media: [
        {
          id: "m-a",
          title: "Film A",
          slug: "film-a",
          files: [],
          ladders: [aSet("film-a")],
          total_bytes: 0,
          created_at: "",
        },
        {
          id: "m-b",
          title: "Film B",
          slug: "film-b",
          files: [],
          ladders: [aSet("film-b")],
          total_bytes: 0,
          created_at: "",
        },
      ],
      unrecognized: [],
      disk: null,
      stale: false,
    });
    renderIn(<ViewersScreen />, "ru");
    await waitFor(() => expect(mockWatchStart).toHaveBeenCalledWith("s1"));
    send?.(update([viewer({ media_id: "m-b" })]));
    expect(await screen.findByText("Film B")).toBeInTheDocument();

    // 1. open it on the viewer; 2. set the cap; 3. agree.
    fireEvent.click(await screen.findByRole("button", { name: ru.ui.limits.title }));
    await waitFor(() => expect(mockLimitPreview).toHaveBeenCalled());
    expect(mockLimitPreview.mock.calls[0][0].slug).toBe("film-b");
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "3" } });
    await waitFor(() => expect(mockLimitPreview).toHaveBeenCalledTimes(2));
    expect(mockLimitPreview.mock.calls[1][0]).toMatchObject({ slug: "film-b" });
    await waitFor(() => expect(screen.getByTestId("confirm")).toBeEnabled());
    fireEvent.click(screen.getByTestId("confirm"));
    await waitFor(() => expect(mockLimitSet).toHaveBeenCalledTimes(1));
    expect((mockLimitSet.mock.calls[0] as unknown[])[0]).toMatchObject({
      ip: "203.0.113.9",
      slug: "film-b",
      cap_bps: 3_000_000,
    });
    // T704: the screen says the rule is written, and when it takes effect — not "done".
    expect(await screen.findByTestId("limit-applied")).toHaveTextContent(
      ru.ui.limits.applied.replace("{ip}", "203.0.113.9"),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
