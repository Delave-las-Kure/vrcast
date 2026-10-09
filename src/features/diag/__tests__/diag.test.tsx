/**
 * T320 — the diagnostics, from a person's side.
 *
 * What is checked is not that the screen renders, but the milestone's three promises:
 *
 * 1. **every reading carries its own verdict** — "worth a look overall" does not say where to
 *    look;
 * 2. **the verdict is shown with the numbers it rests on** (FR-072) — it is sometimes wrong,
 *    and without the numbers there is nothing to argue with;
 * 3. **"could not tell" is a state of its own**, not an empty screen: emptiness is read as
 *    "all is well" or as the application being broken, and both are untrue.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type { Health, Logs, Peaks, Stalls } from "../../../shared/contract";

const mockHealth = vi.fn<() => Promise<Health>>();
const mockLogs = vi.fn<() => Promise<Logs>>();
const mockStalls = vi.fn<(...a: unknown[]) => Promise<Stalls>>();
const mockBitrate = vi.fn<() => Promise<Peaks>>();
const mockOpen = vi.fn<() => Promise<string | null>>();
// The films on the server (T705). Unset — an answer with no media, as the stub gives.
const mockLibrary = vi.fn<() => Promise<unknown>>(async () => []);

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      diagHealth: () => mockHealth(),
      diagLogs: () => mockLogs(),
      diagExplainStalls: (...a: unknown[]) => mockStalls(...a),
      diagBitrate: () => mockBitrate(),
      libraryList: () => mockLibrary(),
    }),
  };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => mockOpen() }));

const { DiagScreen } = await import("../DiagScreen");
const { BitratePeaks } = await import("../BitratePeaks");

const SNAPSHOT: Health["snapshot"] = {
  services: [{ name: "caddy", state: "active" }],
  firewall_status: "active",
  memory: {
    total_mb: 1900,
    used_mb: 400,
    buff_cache_mb: 100,
    swap_total_mb: 1024,
    swap_used_mb: 0,
  },
  disk: { used_mb: 20000, free_mb: 20000 },
  tuning: {
    congestion: "bbr",
    qdisc: "fq",
    slow_start_after_idle: false,
    readahead_kb: 8192,
    restart: "always",
  },
  open_ports: ["0.0.0.0:443"],
  delivery: { Answered: { status: 206 } },
  watching_now: 3,
  container: false,
};

/** Serving down, the cache small, the kernel settings unknowable in a container — all three
 *  verdicts at once. */
const HEALTH: Health = {
  snapshot: SNAPSHOT,
  worst: "trouble",
  readings: [
    {
      about: "serving",
      rating: "trouble",
      say: { key: "HEALTH_SERVING_STOPPED", params: { service: "caddy", state: "failed" } },
    },
    {
      about: "serving_cache",
      rating: "watch",
      say: {
        key: "HEALTH_CACHE_SMALL",
        params: { cache_mb: 100, total_mb: 1900, watching: 3 },
      },
    },
    { about: "network", rating: "unknown", say: { key: "HEALTH_NOT_IN_CONTAINER" } },
    { about: "firewall", rating: "fine", say: { key: "HEALTH_FIREWALL_ON" } },
  ],
};

const LOGS: Logs = {
  reached_the_cap: false,
  oldest: null,
  digest: {
    lines: 101,
    requests: 100,
    unreadable: 1,
    by_status: { "200": 5, "206": 95 },
    addresses: 4,
    top_paths: [{ what: "/videos/film/v30/seg_00001.m4s", times: 20 }],
    top_addresses: [{ what: "203.0.113.24", times: 25 }],
    failures: [],
    long_requests: [
      {
        client_ip: "203.0.113.1",
        path: "/videos/film.mp4",
        seconds: 40,
        bytes: 200_000_000,
        mbit_s: 40,
        slow: false,
      },
    ],
    bytes_out: 300_000_000,
    from: null,
    to: null,
  },
};

const STALLS: Stalls = {
  load: {
    cpu_busy: 0.04,
    disk_read_mb_s: 1,
    out_mbit_s: 18,
    capacity_mbit_s: 940,
    cache_small: false,
  },
  watchers: [
    {
      client_ip: "203.0.113.24",
      watching: "the-recorded-case",
      segments: 20,
      bytes: 300_112_500,
      first: "2026-08-24T00:00:00Z",
      last: "2026-08-24T00:02:31Z",
      elapsed_s: 151,
      content_ratio: 0.53,
      mbit_s: 15.9,
      in_download_mbit_s: 18.6,
      skipped: [10, 12, 13, 15],
      restarts: 2,
      reinits: 1,
      failures: 0,
    },
  ],
  verdicts: [
    {
      cause: "viewer_link",
      say: {
        key: "STALLS_VIEWER_LINK",
        params: {
          ratio: 0.53,
          mbit_s: 15.9,
          in_download_mbit_s: 18.6,
          need_mbit: 10.7,
          skipped: 4,
          restarts: 2,
        },
      },
    },
  ],
  set_aside: [
    { client_ip: "192.0.2.1", why: "our_own_check" },
    { client_ip: "192.0.2.55", why: { too_little: { segments: 2 } } },
  ],
};

describe("the diagnosis screen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockHealth.mockResolvedValue(HEALTH);
    mockLogs.mockResolvedValue(LOGS);
    mockStalls.mockResolvedValue(STALLS);
    mockOpen.mockResolvedValue(null);
  });

  it("marks every reading with its own rating, not one badge for the lot", async () => {
    renderIn(<DiagScreen serverId="s1" />);

    await waitFor(() => expect(screen.getByTestId("reading-serving")).toBeInTheDocument());
    expect(screen.getByTestId("reading-serving")).toHaveAttribute("data-rating", "trouble");
    expect(screen.getByTestId("reading-serving_cache")).toHaveAttribute("data-rating", "watch");
    expect(screen.getByTestId("reading-firewall")).toHaveAttribute("data-rating", "fine");
  });

  it("does not dress up what could not be established as fine", async () => {
    // Otherwise a run in a container would report kernel settings as checked when they
    // cannot be seen there at all — and such a report gets believed.
    renderIn(<DiagScreen serverId="s1" />);

    await waitFor(() => expect(screen.getByTestId("reading-network")).toBeInTheDocument());
    const network = screen.getByTestId("reading-network");
    expect(network).toHaveAttribute("data-rating", "unknown");
    // In words too: markup a person does not read tells them nothing.
    expect(network).toHaveTextContent(ru.ui.diag.ratingUnknown);
    expect(network).not.toHaveTextContent(ru.ui.diag.ratingFine);
    // The reason is named — "this cannot be seen in a container" — rather than left silent.
    expect(network.textContent ?? "").toContain("контейнер");
  });

  it("names the stopped service instead of saying something is down", async () => {
    renderIn(<DiagScreen serverId="s1" />);
    await waitFor(() => expect(screen.getByTestId("reading-serving")).toBeInTheDocument());
    expect(screen.getByTestId("reading-serving")).toHaveTextContent("caddy");
  });

  it("shows the conclusion together with the figures it rests on", async () => {
    renderIn(<DiagScreen serverId="s1" />);

    await waitFor(() => expect(screen.getByTestId("verdict-203.0.113.24")).toBeInTheDocument());
    // The verdict itself, with the numbers inside the sentence...
    const verdict = screen.getByTestId("verdict-203.0.113.24");
    // In the units and separator of the language (T706), not "0.53" and a bare "15.9".
    expect(verdict).toHaveTextContent("0,53×");
    expect(verdict).toHaveTextContent("15,9 Мбит/с");
    expect(verdict).toHaveTextContent("нужно 10,7 Мбит/с");
    expect(verdict.textContent).not.toMatch(/\{\w+\|?\w*\}/);
    // ...and the same numbers apart from it, because viewers get compared down a column by
    // eye.
    expect(screen.getByTestId("ratio-203.0.113.24")).toHaveTextContent("0,53");
    expect(screen.getByTestId("link-203.0.113.24")).toHaveTextContent("15,9");
  });

  it("keeps the viewer's link and the speed inside the downloads apart", async () => {
    // Confusing the two means telling somebody with a perfectly good line to change
    // provider.
    renderIn(<DiagScreen serverId="s1" />);
    await waitFor(() => expect(screen.getByTestId("link-203.0.113.24")).toBeInTheDocument());

    const shown = screen.getByTestId("link-203.0.113.24").textContent ?? "";
    expect(shown).toContain("15,9");
    expect(shown).toContain("18,6");
    expect(shown.indexOf("15,9")).toBeLessThan(shown.indexOf("18,6"));
  });

  it("shows who was not a viewer and why", async () => {
    renderIn(<DiagScreen serverId="s1" />);
    await waitFor(() => expect(screen.getByTestId("aside-192.0.2.1")).toBeInTheDocument());
    expect(screen.getByTestId("aside-192.0.2.55")).toHaveTextContent("2");
  });

  it("says a long request is normally fine rather than flagging it", async () => {
    renderIn(<DiagScreen serverId="s1" />);
    await waitFor(() => expect(screen.getByTestId("logs-long-normal")).toBeInTheDocument());
    expect(screen.getByTestId("long-normal")).toBeInTheDocument();
    expect(screen.queryByTestId("long-slow")).not.toBeInTheDocument();
  });

  it("has a state of its own for what could not be determined", async () => {
    mockHealth.mockRejectedValue({ code: "SSH_UNREACHABLE", details: [] });
    renderIn(<DiagScreen serverId="s1" />);

    // Not an empty screen: emptiness is read as "all is well" or as the application being
    // broken.
    await waitFor(() => expect(screen.queryByTestId("diag-asking")).not.toBeInTheDocument());
    expect(screen.queryByTestId("reading-serving")).not.toBeInTheDocument();
    expect(document.body.textContent?.trim().length ?? 0).toBeGreaterThan(0);
  });

  it("re-asks why a viewer stalls with the file's shape once it is measured", async () => {
    // T500. `diag_explain_stalls` reaches `Cause::TheFileItself` and `Cause::ThePlayer`
    // only with `Some(file)` — without it every stalling viewer falls into the general
    // "not enough channel" verdict. `BitratePeaks` already measures the file; this screen
    // used to never pass what it found along.
    mockOpen.mockResolvedValue("F:/films/film.mp4");
    mockBitrate.mockResolvedValue({
      average_bps: 8_000_000,
      median_bps: 7_500_000,
      one_second: { at_s: 10, length_s: 1, bitrate_bps: 20_000_000 },
      wide: { at_s: 100, length_s: 10, bitrate_bps: 41_000_000 },
      worst_wide: [],
      seconds: 3600,
    });
    renderIn(<DiagScreen serverId="s1" />);

    // The first, automatic call — before anything has been measured — carries no file.
    await waitFor(() => expect(mockStalls).toHaveBeenCalledTimes(1));
    expect(mockStalls.mock.calls[0]).toEqual(["s1", 30, undefined]);

    fireEvent.click(await screen.findByText(ru.ui.diag.bitratePick));

    // Measuring the file re-runs the stall reading, this time with its shape — converted
    // from bits/s to megabits, as `LadderScreen.tsx`'s `bitrate()` already does.
    await waitFor(() => expect(mockStalls).toHaveBeenCalledTimes(2));
    expect(mockStalls.mock.calls[1]).toEqual([
      "s1",
      30,
      { average_mbit: 8, peak_10s_mbit: 41, slug: null },
    ]);
  });
});

describe("T594 — DiagScreen ignores a stale health answer after a quick period switch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockHealth.mockResolvedValue(HEALTH);
    mockLogs.mockResolvedValue(LOGS);
    mockStalls.mockResolvedValue(STALLS);
    mockOpen.mockResolvedValue(null);
  });

  it("keeps the second request's data when the first, slower one resolves later", async () => {
    // The first, automatic `ask()` (triggered on mount) hangs on `diagHealth` — the same
    // technique as T592's own test in `deploy.test.tsx` (`resolveKeep`).
    let resolveFirst: (h: Health) => void = () => {};
    mockHealth.mockImplementationOnce(
      () =>
        new Promise<Health>((resolve) => {
          resolveFirst = resolve;
        }),
    );
    renderIn(<DiagScreen serverId="s1" />);
    await waitFor(() => expect(mockHealth).toHaveBeenCalledTimes(1));

    // The second call — after switching the period — resolves quickly, with data
    // distinguishable by content from the first (still-hanging) call's would-be answer.
    const SECOND: Health = {
      ...HEALTH,
      worst: "fine",
      readings: [
        {
          about: "serving",
          rating: "fine",
          say: { key: "HEALTH_SERVING_STOPPED", params: { service: "nginx", state: "active" } },
        },
      ],
    };
    mockHealth.mockResolvedValueOnce(SECOND);

    fireEvent.change(screen.getByTestId("diag-period"), { target: { value: "10" } });
    await waitFor(() => expect(mockHealth).toHaveBeenCalledTimes(2));

    // The second (current) request's answer must be on screen...
    await waitFor(() => expect(screen.getByTestId("reading-serving")).toHaveTextContent("nginx"));
    expect(screen.getByTestId("reading-serving")).toHaveAttribute("data-rating", "fine");

    // ...and once the first, now-stale request finally answers, it must not overwrite it:
    // by the time it resolves, it is no longer the request the period select is showing.
    // A stale answer that failed the `gen` check never reaches `diagLogs` either — it
    // returns right after `setHealth` would have run — so `mockLogs` stays at the one
    // call the second (current) request made.
    resolveFirst(HEALTH);
    await waitFor(() => expect(screen.getByTestId("reading-serving")).toBeInTheDocument());
    expect(screen.getByTestId("reading-serving")).toHaveTextContent("nginx");
    expect(screen.getByTestId("reading-serving")).toHaveAttribute("data-rating", "fine");
    expect(mockLogs).toHaveBeenCalledTimes(1);
  });
});

/**
 * T669 — QA-24B-10. Measuring a long file A, then choosing B: B's measurement finished first,
 * and A's arrived afterwards and was taken as the answer — B's name on screen with A's
 * figures, and A's shape handed to the diagnosis of B's stalls.
 */
describe("T669 — a late measurement of an earlier file does not replace the current one", () => {
  function peaks(mbit: number): Peaks {
    return {
      seconds: 600,
      average_bps: mbit * 1_000_000,
      median_bps: mbit * 1_000_000,
      one_second: { at_s: 2, length_s: 1, bitrate_bps: mbit * 1_000_000 },
      wide: { at_s: 0, length_s: 10, bitrate_bps: mbit * 1_000_000 },
      worst_wide: [],
    };
  }

  function held<T>() {
    let resolve: (v: T) => void = () => undefined;
    let reject: (e: unknown) => void = () => undefined;
    const promise = new Promise<T>((ok, fail) => {
      resolve = ok;
      reject = fail;
    });
    return { promise, resolve, reject };
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("keeps the figures and the shape of the file chosen last", async () => {
    const a = held<Peaks>();
    const b = held<Peaks>();
    mockOpen.mockResolvedValueOnce("F:/qa/a.mp4").mockResolvedValueOnce("F:/qa/b.mp4");
    mockBitrate.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    const measured = vi.fn();
    renderIn(<BitratePeaks onMeasured={measured} />);

    fireEvent.click(screen.getByRole("button", { name: ru.ui.diag.bitratePick }));
    await waitFor(() => expect(mockBitrate).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: ru.ui.diag.bitratePick }));
    await waitFor(() => expect(mockBitrate).toHaveBeenCalledTimes(2));

    b.resolve(peaks(2));
    await waitFor(() =>
      expect(measured).toHaveBeenLastCalledWith({ average_mbit: 2, peak_10s_mbit: 2, slug: null }),
    );
    a.resolve(peaks(50));
    await new Promise((r) => setTimeout(r, 0));

    expect(screen.getByText("F:/qa/b.mp4")).toBeInTheDocument();
    expect(measured).toHaveBeenLastCalledWith({ average_mbit: 2, peak_10s_mbit: 2, slug: null });
    expect(measured).not.toHaveBeenCalledWith({ average_mbit: 50, peak_10s_mbit: 50, slug: null });
    expect(screen.getByTestId("bitrate-average").textContent).not.toMatch(/50/);
  });

  it("an earlier file's failure arriving late does not blank the current one's figures", async () => {
    const a = held<Peaks>();
    const b = held<Peaks>();
    mockOpen.mockResolvedValueOnce("F:/qa/a.mp4").mockResolvedValueOnce("F:/qa/b.mp4");
    mockBitrate.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    const measured = vi.fn();
    renderIn(<BitratePeaks onMeasured={measured} />);

    fireEvent.click(screen.getByRole("button", { name: ru.ui.diag.bitratePick }));
    await waitFor(() => expect(mockBitrate).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: ru.ui.diag.bitratePick }));
    await waitFor(() => expect(mockBitrate).toHaveBeenCalledTimes(2));

    b.resolve(peaks(3));
    await waitFor(() => expect(screen.getByTestId("bitrate-average")).toBeInTheDocument());
    a.reject({ code: "INTERNAL" });
    await new Promise((r) => setTimeout(r, 0));

    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByTestId("bitrate-average")).toBeInTheDocument();
    expect(measured).toHaveBeenLastCalledWith({ average_mbit: 3, peak_10s_mbit: 3, slug: null });
  });

  it("while the current file is still being measured, an earlier answer says nothing at all", async () => {
    const a = held<Peaks>();
    const b = held<Peaks>();
    mockOpen.mockResolvedValueOnce("F:/qa/a.mp4").mockResolvedValueOnce("F:/qa/b.mp4");
    mockBitrate.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    const measured = vi.fn();
    renderIn(<BitratePeaks onMeasured={measured} />);

    fireEvent.click(screen.getByRole("button", { name: ru.ui.diag.bitratePick }));
    await waitFor(() => expect(mockBitrate).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: ru.ui.diag.bitratePick }));
    await waitFor(() => expect(mockBitrate).toHaveBeenCalledTimes(2));

    a.resolve(peaks(50));
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByTestId("bitrate-average")).toBeNull();
    expect(screen.getByText(ru.ui.diag.asking)).toBeInTheDocument();
    expect(measured).not.toHaveBeenCalledWith({ average_mbit: 50, peak_10s_mbit: 50, slug: null });
  });
});

/**
 * T705 (QA-26 №12) — conclusions tied to a particular film: a file measured on this computer
 * is one film, said or guessed, and a viewer is named by the film they watch.
 */
describe("T705 — the diagnosis is about a particular film", () => {
  const LIBRARY = {
    server_id: "s1",
    media: [
      { id: "m1", title: "The Recorded Case", slug: "the-recorded-case", files: [], ladders: [] },
      { id: "m2", title: "Another", slug: "another", files: [], ladders: [] },
    ],
    unrecognized: [],
    disk: null,
    stale: false,
  };
  const PEAKS: Peaks = {
    average_bps: 8_000_000,
    median_bps: 7_500_000,
    one_second: { at_s: 10, length_s: 1, bitrate_bps: 20_000_000 },
    wide: { at_s: 100, length_s: 10, bitrate_bps: 41_000_000 },
    worst_wide: [],
    seconds: 3600,
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mockHealth.mockResolvedValue(HEALTH);
    mockLogs.mockResolvedValue(LOGS);
    mockStalls.mockResolvedValue({
      ...STALLS,
      watchers: [{ ...STALLS.watchers[0], rung: "v9", need_mbit: 10.7 }],
    });
    mockLibrary.mockResolvedValue(LIBRARY);
    mockBitrate.mockResolvedValue(PEAKS);
  });

  it("names the viewer's film by its title, and what that quality needs", async () => {
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() =>
      expect(screen.getByTestId("watcher-203.0.113.24")).toHaveTextContent("The Recorded Case"),
    );
    expect(screen.getByTestId("needs-203.0.113.24")).toHaveTextContent("10,7 Мбит/с");
  });

  it("a file named like a film on the server is taken for that film", async () => {
    mockOpen.mockResolvedValue("F:/films/The Recorded Case.mkv");
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(mockStalls).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockLibrary).toHaveBeenCalled());
    fireEvent.click(await screen.findByText(ru.ui.diag.bitratePick));
    await waitFor(() => expect(mockStalls).toHaveBeenCalledTimes(2));
    expect(mockStalls.mock.calls[1][2]).toEqual({
      average_mbit: 8,
      peak_10s_mbit: 41,
      slug: "the-recorded-case",
    });
    expect((screen.getByTestId("bitrate-film") as HTMLSelectElement).value).toBe(
      "the-recorded-case",
    );
  });

  it("a file the name of no film judges nobody until the person says which film it is", async () => {
    mockOpen.mockResolvedValue("F:/films/rip-1080p.mkv");
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(mockLibrary).toHaveBeenCalled());
    fireEvent.click(await screen.findByText(ru.ui.diag.bitratePick));
    await waitFor(() => expect(mockStalls).toHaveBeenCalledTimes(2));
    expect(mockStalls.mock.calls[1][2]).toMatchObject({ slug: null });

    fireEvent.change(screen.getByTestId("bitrate-film"), { target: { value: "another" } });
    await waitFor(() => expect(mockStalls).toHaveBeenCalledTimes(3));
    expect(mockStalls.mock.calls[2][2]).toMatchObject({ slug: "another" });
  });
});

/** T706 (QA-26 №11, tour G) — the diagnosis says only what it knows, in the person's units. */
describe("T706 — the diagnosis in plain units, and nothing about seeking from a set's log", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockHealth.mockResolvedValue(HEALTH);
    mockStalls.mockResolvedValue(STALLS);
    mockLibrary.mockResolvedValue([]);
  });

  it("a set's log answered 200 throughout says nothing about seeking", async () => {
    // A healthy quality set: whole segments, every one rightly answered 200.
    mockLogs.mockResolvedValue({
      ...LOGS,
      digest: { ...LOGS.digest, by_status: { "200": 9122, "206": 10 } },
    });
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByTestId("status-200")).toBeInTheDocument());
    expect(screen.queryByTestId("logs-ranges")).toBeNull();
    expect(document.body.textContent).not.toContain("перемотка не работает");
  });

  it("long requests and the disk are in Russian units", async () => {
    mockLogs.mockResolvedValue(LOGS);
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByTestId("long-normal")).toBeInTheDocument());
    expect(screen.getByTestId("long-normal")).toHaveTextContent("40 с ·");
    expect(screen.getByTestId("stalls-load")).toHaveTextContent("1 МБ/с");
    expect(screen.getByTestId("stalls-load").textContent).not.toContain("MB/s");
  });

  it("the bitrate peaks say they measure the file on this computer, and a window at 0:00 has a time", async () => {
    mockLogs.mockResolvedValue(LOGS);
    mockOpen.mockResolvedValue("F:/films/a.mkv");
    mockBitrate.mockResolvedValue({
      average_bps: 26_900_000,
      median_bps: 8_000_000,
      one_second: { at_s: 43, length_s: 1, bitrate_bps: 66_600_000 },
      wide: { at_s: 47, length_s: 10, bitrate_bps: 65_900_000 },
      worst_wide: [
        { at_s: 47, length_s: 10, bitrate_bps: 65_900_000 },
        { at_s: 0, length_s: 10, bitrate_bps: 7_000_000 },
      ],
      seconds: 120,
    });
    renderIn(<DiagScreen serverId="s1" />, "ru");
    expect(screen.getByTestId("bitrate-where")).toHaveTextContent(ru.ui.diag.bitrateWhere);
    fireEvent.click(screen.getByText(ru.ui.diag.bitratePick));
    await waitFor(() => expect(screen.getByTestId("window-0")).toBeInTheDocument());
    expect(screen.getByTestId("window-0")).toHaveTextContent("0:00 — 7,0 Мбит/с");
    expect(document.body.textContent).toContain("в 2,4 раза");
  });

  it("is called Diagnostics in English, in the menu and on the screen alike", () => {
    expect(en.ui.diag.title).toBe("Diagnostics");
    expect(en.ui.diag.title).toBe(en.ui.sections.diagnostics);
  });
});

/** T711 — the viewer's own connection, read live, tells their link from their player. */
describe("T711 — the link and the player told apart on the live connection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockHealth.mockResolvedValue(HEALTH);
    mockLogs.mockResolvedValue(LOGS);
    mockLibrary.mockResolvedValue([]);
  });

  it("shows what the connection carries now, and the verdict made from it", async () => {
    mockStalls.mockResolvedValue({
      ...STALLS,
      watchers: [
        {
          ...STALLS.watchers[0],
          need_mbit: 2,
          live: {
            span_s: 5,
            mbit_s: 0.4,
            busy_mbit_s: 0.4,
            busy_share: 1,
            held_share: 0,
            resent_share: 0.15,
          },
        },
      ],
      verdicts: [
        {
          cause: "viewer_link",
          say: {
            key: "STALLS_VIEWER_LINK_LIVE",
            params: { ratio: 0.4, live_mbit: 0.4, need_mbit: 2, resent_pct: 15 },
          },
        },
      ],
    });
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByTestId("live-203.0.113.24")).toBeInTheDocument());
    expect(screen.getByTestId("live-203.0.113.24")).toHaveTextContent("400 кбит/с");
    const verdict = screen.getByTestId("verdict-203.0.113.24");
    expect(verdict).toHaveTextContent("сейчас несёт 400 кбит/с при нужных 2,0 Мбит/с");
    expect(verdict).toHaveTextContent("Отправлено повторно: 15%");
    expect(verdict.textContent).not.toMatch(/\{\w+/);
  });

  it("without a live connection there is no live figure at all", async () => {
    mockStalls.mockResolvedValue(STALLS);
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByTestId("verdict-203.0.113.24")).toBeInTheDocument());
    expect(screen.queryByTestId("live-203.0.113.24")).toBeNull();
  });

  it("every live verdict is said in both languages with the same figures", () => {
    for (const key of ["STALLS_PLAYER_LIVE", "STALLS_LINK_FINE_LIVE", "STALLS_VIEWER_LINK_LIVE"]) {
      const names = (s: string) => [...s.matchAll(/\{(\w+)/g)].map((m) => m[1]).sort();
      const r = (ru.details as Record<string, string>)[key];
      const e = (en.details as Record<string, string>)[key];
      expect(r, key).toBeTruthy();
      expect(names(r)).toEqual(names(e));
    }
  });
});

/** T712 — the server's link weighed against the plan on its card, and said to be. */
describe("T712 — the plan, not the network card", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockHealth.mockResolvedValue(HEALTH);
    mockLogs.mockResolvedValue(LOGS);
    mockLibrary.mockResolvedValue([]);
  });

  it("says the capacity is the plan's when it is", async () => {
    mockStalls.mockResolvedValue({
      ...STALLS,
      load: { ...STALLS.load, capacity_mbit_s: 100, capacity_by: "tariff" },
    });
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByTestId("stalls-load")).toBeInTheDocument());
    expect(screen.getByTestId("stalls-load")).toHaveTextContent(
      "из положенных по тарифу 100,0 Мбит/с",
    );
    expect(screen.getByTestId("stalls-load").textContent).not.toContain("сетевой карте");
  });

  it("and the network card's when no plan was written", async () => {
    mockStalls.mockResolvedValue(STALLS);
    renderIn(<DiagScreen serverId="s1" />, "ru");
    await waitFor(() => expect(screen.getByTestId("stalls-load")).toBeInTheDocument());
    expect(screen.getByTestId("stalls-load")).toHaveTextContent("по сетевой карте");
  });
});
