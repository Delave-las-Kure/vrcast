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

import { screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { Health, Logs, Stalls } from "../../../shared/contract";

const mockHealth = vi.fn<() => Promise<Health>>();
const mockLogs = vi.fn<() => Promise<Logs>>();
const mockStalls = vi.fn<() => Promise<Stalls>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      diagHealth: () => mockHealth(),
      diagLogs: () => mockLogs(),
      diagExplainStalls: () => mockStalls(),
      diagBitrate: () => Promise.reject(new Error("not asked for in these checks")),
    },
  };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));

const { DiagScreen } = await import("../DiagScreen");

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
    expect(verdict).toHaveTextContent("0.53");
    expect(verdict).toHaveTextContent("15.9");
    // ...and the same numbers apart from it, because viewers get compared down a column by
    // eye.
    expect(screen.getByTestId("ratio-203.0.113.24")).toHaveTextContent("0.53");
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
});
