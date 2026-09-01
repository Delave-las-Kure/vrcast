/**
 * T220 — capping a viewer's quality, from the interface.
 *
 * What is checked is what these two screens exist to prevent: a change to a live serving
 * configuration made without a word, a person choosing a cap without seeing what it leaves
 * them, and a list of limits that says something the server does not.
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type { LimitPreview, QualityLimit } from "../../../shared/contract";

const mockPreview = vi.fn<() => Promise<LimitPreview>>();
const mockSet = vi.fn<(...a: unknown[]) => Promise<void>>();
const mockClear = vi.fn<(...a: unknown[]) => Promise<void>>();
const mockList = vi.fn<() => Promise<QualityLimit[]>>();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      limitPreview: () => mockPreview(),
      limitSet: (...a: unknown[]) => mockSet(...a),
      limitClear: (...a: unknown[]) => mockClear(...a),
      limitsList: () => mockList(),
    }),
  };
});

const { LimitDialog } = await import("../LimitDialog");
const { LimitsList } = await import("../LimitsList");

const MEDIA = [{ slug: "demo", title: "Demo film" }];

function variant(bandwidth: number, height: number) {
  return {
    path: `/videos/demo/v${Math.round(bandwidth / 1_000_000)}/stream.m3u8`,
    bandwidth,
    average_bandwidth: Math.round(bandwidth * 0.8),
    width: Math.round((height * 16) / 9),
    height,
    fps: 24,
    codecs: "avc1.640029,mp4a.40.2",
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockSet.mockResolvedValue(undefined);
  mockClear.mockResolvedValue(undefined);
  mockPreview.mockResolvedValue({
    kept: [variant(6_000_000, 1080), variant(3_000_000, 720)],
    warnings: [{ key: "WARN_LIMIT_FOLLOWS_THE_ADDRESS", params: {} }],
    below_lightest: false,
  });
});

describe("before the cap goes on", () => {
  it("shows what the viewer would be left with", async () => {
    // A person choosing a cap is choosing from what it leaves, not from a number.
    renderIn(<LimitDialog serverId="s1" ip="203.0.113.10" media={MEDIA} />, "en");
    await waitFor(() => expect(screen.getByTestId("kept")).toBeInTheDocument());
    expect(screen.getByTestId("kept")).toHaveTextContent("6.0 Mbit/s");
    expect(screen.getByTestId("kept")).toHaveTextContent("3.0 Mbit/s");
  });

  it("shows every warning, and shows it before the button", async () => {
    // A warning shown afterwards is a report, and a report about something already done is
    // of no use to anybody.
    renderIn(<LimitDialog serverId="s1" ip="203.0.113.10" media={MEDIA} />, "en");
    await waitFor(() =>
      expect(screen.getByTestId("warnings")).toHaveTextContent(
        en.details.WARN_LIMIT_FOLLOWS_THE_ADDRESS,
      ),
    );
    expect(mockSet).not.toHaveBeenCalled();
  });

  it("says when several people are behind the address", async () => {
    // Ordinary for a household or an office, and the cap reaches all of them.
    mockPreview.mockResolvedValue({
      kept: [variant(3_000_000, 720)],
      warnings: [
        { key: "WARN_LIMIT_FOLLOWS_THE_ADDRESS", params: {} },
        { key: "WARN_ADDRESS_SHARED", params: { count: 3 } },
      ],
      below_lightest: false,
    });
    renderIn(<LimitDialog serverId="s1" ip="203.0.113.10" media={MEDIA} />, "en");
    await waitFor(() => expect(screen.getByTestId("warnings")).toHaveTextContent("3 viewers"));
  });

  it("says when the cap is below anything that exists, and still offers the lightest", async () => {
    // FR-067. An empty description would leave the viewer with no video at all.
    mockPreview.mockResolvedValue({
      kept: [variant(3_000_000, 720)],
      warnings: [
        { key: "WARN_LIMIT_FOLLOWS_THE_ADDRESS", params: {} },
        { key: "WARN_CAP_BELOW_LIGHTEST", params: { lightest_bps: 3_000_000 } },
      ],
      below_lightest: true,
    });
    renderIn(<LimitDialog serverId="s1" ip="203.0.113.10" media={MEDIA} />, "ru");
    await waitFor(() => expect(screen.getByTestId("kept")).toHaveTextContent("3.0"));
    // Asked of the catalogue rather than copied out of it: a sentence written into a test
    // breaks the day somebody improves the wording, and then says nothing about what is
    // actually wrong.
    expect(screen.getByTestId("warnings")).toHaveTextContent(
      ru.details.WARN_CAP_BELOW_LIGHTEST.split("(")[0].trim(),
    );
  });
});

describe("putting the cap on", () => {
  it("only happens when the person agrees, and says so to the core", async () => {
    // The core refuses an unconfirmed change of its own accord; the interface must not be
    // the only thing standing between a slip of the mouse and a live configuration.
    renderIn(<LimitDialog serverId="s1" ip="203.0.113.10" media={MEDIA} />, "en");
    await waitFor(() => expect(screen.getByTestId("confirm")).toBeEnabled());
    fireEvent.click(screen.getByTestId("confirm"));

    await waitFor(() => expect(mockSet).toHaveBeenCalledTimes(1));
    expect(mockSet.mock.calls[0][1]).toBe(true);
    // snake_case, because that is what the core reads. This said `serverId` until
    // 2026-08-28 and agreed with the screen, and the two of them agreed on a shape the
    // core refuses outright — `missing field server_id`. Capping a viewer never once
    // worked, and this test was green throughout.
    expect(mockSet.mock.calls[0][0]).toMatchObject({
      server_id: "s1",
      ip: "203.0.113.10",
      slug: "demo",
      cap_bps: 6_000_000,
    });
  });

  it("cannot be pressed until there is something to agree to", async () => {
    // Until the preview arrives there is nothing on screen to have understood.
    mockPreview.mockImplementation(() => new Promise(() => undefined));
    renderIn(<LimitDialog serverId="s1" ip="203.0.113.10" media={MEDIA} />, "en");
    expect(screen.getByTestId("confirm")).toBeDisabled();
  });
});

describe("what is capped now", () => {
  it("lists what the server says, and lifting one asks the server again", async () => {
    // FR-064, FR-065. Struck off the list here instead, the interface would show a state
    // nobody had confirmed — and it is exactly when a change half-fails that a person needs
    // to be told the truth.
    mockList
      .mockResolvedValueOnce([
        { ip: "203.0.113.10", slug: "demo", cap_bps: 6_000_000, set_at: "2026-08-26T10:00:00Z" },
      ])
      .mockResolvedValueOnce([]);
    renderIn(<LimitsList serverId="s1" />, "en");

    await waitFor(() =>
      expect(screen.getByTestId("limit-203.0.113.10/demo")).toHaveTextContent("6.0 Mbit/s"),
    );
    fireEvent.click(screen.getByText(en.ui.limits.remove));

    await waitFor(() => expect(mockClear).toHaveBeenCalledTimes(1));
    expect(mockClear.mock.calls[0]).toEqual(["s1", "203.0.113.10", "demo"]);
    await waitFor(() => expect(screen.getByTestId("no-limits")).toBeInTheDocument());
    expect(mockList).toHaveBeenCalledTimes(2);
  });

  it("an empty list says so rather than showing nothing at all", async () => {
    mockList.mockResolvedValue([]);
    renderIn(<LimitsList serverId="s1" />, "ru");
    await waitFor(() =>
      expect(screen.getByTestId("no-limits")).toHaveTextContent(ru.ui.limits.listEmpty),
    );
  });
});
