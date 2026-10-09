/**
 * T673 — the «Video» screen: one place from a file to a link.
 *
 * What is checked is what the owner asked for: a video goes the whole way on one card, every
 * stage is visible, nothing has to be found again and handed to the next screen, and the
 * screen comes back as it was after a restart — because it is built from `videoList` alone.
 */

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type {
  AppError,
  Rung,
  ServerProfile,
  SourceFile,
  VideoPlan,
  VideoProblemAction,
  VideoView,
} from "../../../shared/contract";

const mockVideoList = vi.fn<() => Promise<VideoView[]>>();
const mockVideoAdd = vi.fn();
const mockVideoStart = vi.fn();
const mockVideoPause = vi.fn();
const mockVideoResume = vi.fn();
const mockVideoCancel = vi.fn();
const mockVideoRetry = vi.fn();
const mockVideoReplace = vi.fn();
const mockVideoRemove = vi.fn();
const mockVideoSetAudio = vi.fn();
const mockVideoSetName = vi.fn();
const mockVideoSetRungs = vi.fn();
const mockLadderRecomputeRung = vi.fn();
const mockServersList = vi.fn<() => Promise<ServerProfile[]>>();
const mockOpen = vi.fn<(options?: unknown) => Promise<string[] | string | null>>();

/** The screen's own `video:update` listener, held so a test can send an event. */
let push: ((v: VideoView) => void) | null = null;
/** The screen's own `video:removed` listener (T683). */
let pushRemoved: ((id: string) => void) | null = null;

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (o: unknown) => mockOpen(o) }));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      serversList: () => mockServersList(),
      videoList: () => mockVideoList(),
      videoAdd: (...a: unknown[]) => mockVideoAdd(...a),
      videoStart: (...a: unknown[]) => mockVideoStart(...a),
      videoPause: (...a: unknown[]) => mockVideoPause(...a),
      videoResume: (...a: unknown[]) => mockVideoResume(...a),
      videoCancel: (...a: unknown[]) => mockVideoCancel(...a),
      videoRetry: (...a: unknown[]) => mockVideoRetry(...a),
      videoReplace: (...a: unknown[]) => mockVideoReplace(...a),
      videoRemove: (...a: unknown[]) => mockVideoRemove(...a),
      videoSetAudio: (...a: unknown[]) => mockVideoSetAudio(...a),
      videoSetName: (...a: unknown[]) => mockVideoSetName(...a),
      videoSetRungs: (...a: unknown[]) => mockVideoSetRungs(...a),
      ladderValidate: () => Promise.resolve({ objections: [], not_buildable: null }),
      ladderRecomputeRung: (...a: unknown[]) => mockLadderRecomputeRung(...a),
    }),
    onVideoUpdate: async (handler: (v: VideoView) => void) => {
      push = handler;
      return () => {
        if (push === handler) push = null;
      };
    },
    onVideoRemoved: async (handler: (id: string) => void) => {
      pushRemoved = handler;
      return () => {
        if (pushRemoved === handler) pushRemoved = null;
      };
    },
  };
});

const { VideoScreen } = await import("../VideoScreen");
const { useServers } = await import("../../servers/store");

function profile(): ServerProfile {
  return {
    id: "srv_1",
    name: "Мой сервер",
    host: "203.0.113.10",
    port: 22,
    user: "root",
    auth_kind: "key",
    secret_ref: "server/srv_1",
    key_path: null,
    domain: "stream.example.com",
    video_dir: "/srv/video",
    cdn_base: null,
    host_fingerprint: "SHA256:aaa",
    ipv6_mode: null,
    is_active: true,
  } as ServerProfile;
}

function rung(index: number, mbps: number, height: number): Rung {
  return {
    index,
    bitrate_bps: mbps * 1_000_000,
    maxrate_bps: mbps * 1_100_000,
    bufsize_bps: mbps * 1_100_000,
    width: Math.round((height * 16) / 9),
    height,
    level: "5.1",
    reasons: ["measured_optimum"],
    quality: { state: "measured_here", vmaf_x100: 9500 },
  };
}

function source(tracks = 1): SourceFile {
  return {
    path: "F:/films/Фильм.mkv",
    size_bytes: 20e9,
    duration_s: 7200,
    width: 3840,
    height: 2160,
    fps: 24,
    bitrate_bps: 40_000_000,
    peak_bps: null,
    video_codec: "h264",
    pix_fmt: "yuv420p",
    color_transfer: null,
    audio_tracks: Array.from({ length: tracks }, (_, i) => ({
      index: i,
      codec: "aac",
      profile: "LC",
      channels: i === 0 ? 6 : 2,
      bitrate_bps: null,
      language: i === 0 ? "rus" : "eng",
      title: null,
      is_default: i === 0,
    })),
    subtitle_tracks: [],
  };
}

function plan(over: Partial<VideoPlan> = {}): VideoPlan {
  return {
    rungs: [rung(0, 8, 1080), rung(1, 4, 720)],
    from: "measured",
    needs_measuring: false,
    measure_s: 0,
    encode_s: 1500,
    encode_estimate: "this_machine",
    encoder: "h264_nvenc",
    server_bytes: 6 * 1024 ** 3,
    local_bytes: 3 * 1024 ** 3,
    server_space: { state: "fits", needed_bytes: 1, free_bytes: 2 },
    local_space: { state: "fits", needed_bytes: 1, free_bytes: 2 },
    name_taken: false,
    objections: [],
    notices: [],
    ...over,
  };
}

function video(over: Partial<VideoView> = {}): VideoView {
  return {
    id: "v1",
    server_id: "srv_1",
    source_path: "F:/films/Фильм.mkv",
    title: "Фильм",
    slug: "film",
    audio_track: 0,
    subtitle_track: null,
    stage: "planned",
    state: "ready",
    paused_by_person: false,
    start_requested: false,
    source: source(),
    plan: plan(),
    progress: null,
    task_id: null,
    media_id: null,
    problem: null,
    link: null,
    created_at: "2026-10-01T10:00:00Z",
    updated_at: "2026-10-01T10:00:00Z",
    rev: 1,
    ...over,
  };
}

function working(over: Partial<VideoView> = {}): VideoView {
  return video({
    stage: "encoding",
    state: "working",
    start_requested: true,
    media_id: "m1",
    progress: {
      task_state: "running",
      progress: 0.42,
      speed_bps: null,
      eta_s: 600,
      rung: 2,
      rungs: 3,
    },
    ...over,
  });
}

function problem(code: AppError["code"], actions: VideoProblemAction[]): VideoView {
  return video({
    stage: "encoding",
    state: "problem",
    start_requested: true,
    problem: { error: { code, details: [], cause: "ssh: broken pipe" }, actions },
  });
}

function show(lang: "ru" | "en" = "ru", path = "/video") {
  return renderIn(
    <MemoryRouter initialEntries={[path]}>
      <VideoScreen />
    </MemoryRouter>,
    lang,
  );
}

async function card(id = "v1") {
  return within(await screen.findByTestId(`video-${id}`));
}

beforeEach(() => {
  vi.clearAllMocks();
  push = null;
  pushRemoved = null;
  useServers.setState({ profiles: [profile()], loading: false, error: null });
  mockServersList.mockResolvedValue([profile()]);
  mockVideoList.mockResolvedValue([]);
  mockOpen.mockResolvedValue(null);
  for (const m of [
    mockVideoPause,
    mockVideoResume,
    mockVideoCancel,
    mockVideoRetry,
    mockVideoReplace,
    mockVideoSetAudio,
    mockVideoSetName,
    mockVideoSetRungs,
  ]) {
    m.mockImplementation((id: string) => Promise.resolve(video({ id })));
  }
  mockVideoRemove.mockResolvedValue(null);
  mockVideoStart.mockImplementation((ids: string[]) =>
    Promise.resolve(ids.map((id) => ({ id, error: null }))),
  );
});

describe("the list", () => {
  it("is put back from videoList after a restart, each video at the stage it was at", async () => {
    mockVideoList.mockResolvedValue([
      working({ id: "a", title: "Первый" }),
      video({
        id: "b",
        title: "Второй",
        stage: "uploading",
        state: "paused",
        paused_by_person: true,
        media_id: "m2",
        progress: {
          task_state: "paused",
          progress: 0.5,
          speed_bps: null,
          eta_s: null,
          rung: 1,
          rungs: 2,
        },
      }),
      video({
        id: "c",
        title: "Третий",
        stage: "done",
        state: "done",
        media_id: "m3",
        link: { origin: "https://stream.example.com/c/master.m3u8", cdn: null },
      }),
    ]);
    show();

    const a = await card("a");
    expect(a.getByText("Первый")).toBeInTheDocument();
    const stagesA = a.getByRole("list", { name: ru.ui.video.stagesLabel });
    expect(within(stagesA).getByText(/Замер/).getAttribute("data-mark")).toBe("passed");
    expect(
      within(stagesA)
        .getByText(/Кодирование/)
        .getAttribute("data-mark"),
    ).toBe("current");
    expect(
      within(stagesA)
        .getByText(/Заливка/)
        .getAttribute("data-mark"),
    ).toBe("ahead");

    const b = await card("b");
    expect(b.getByText(ru.ui.video.paused)).toBeInTheDocument();
    expect(b.getByRole("button", { name: ru.ui.video.resume })).toBeInTheDocument();
    expect(
      within(b.getByRole("list", { name: ru.ui.video.stagesLabel })).getByText(/Заливка/),
    ).toHaveAttribute("data-mark", "current");

    const c = await card("c");
    expect(c.getByText("https://stream.example.com/c/master.m3u8")).toBeInTheDocument();
    expect(mockVideoList).toHaveBeenCalledTimes(1);
  });

  it("follows video:update without asking for the list again", async () => {
    mockVideoList.mockResolvedValue([working()]);
    show();
    await card();
    await waitFor(() => expect(push).not.toBeNull());

    act(() =>
      push!(
        working({
          stage: "uploading",
          updated_at: "2026-10-01T10:05:00Z",
          rev: 5,
          progress: {
            task_state: "running",
            progress: 0.1,
            speed_bps: 5 * 1024 * 1024,
            eta_s: 90,
            rung: 2,
            rungs: 3,
          },
        }),
      ),
    );

    const c = await card();
    await waitFor(() =>
      expect(
        within(c.getByRole("list", { name: ru.ui.video.stagesLabel })).getByText(/Заливка/),
      ).toHaveAttribute("data-mark", "current"),
    );
    expect(c.getByTestId("stage-facts")).toHaveTextContent("5,0 МБ/с");
    expect(mockVideoList).toHaveBeenCalledTimes(1);
  });

  it("does not let an older answer overwrite a newer event", async () => {
    mockVideoList.mockResolvedValue([working()]);
    show();
    await card();
    await waitFor(() => expect(push).not.toBeNull());
    act(() => push!(working({ stage: "cutting", updated_at: "2026-10-01T11:00:00Z", rev: 11 })));
    act(() => push!(working({ stage: "encoding", updated_at: "2026-10-01T10:30:00Z", rev: 10 })));

    const c = await card();
    expect(
      within(c.getByRole("list", { name: ru.ui.video.stagesLabel })).getByText(/Нарезка/),
    ).toHaveAttribute("data-mark", "current");
  });
});

describe("adding videos", () => {
  it("takes several files at once and says per file why one was refused", async () => {
    mockOpen.mockResolvedValue(["F:/films/a.mkv", "F:/films/notes.txt"]);
    mockVideoAdd.mockResolvedValue({
      added: [
        video({
          id: "a",
          title: "a",
          source_path: "F:/films/a.mkv",
          state: "planning",
          plan: null,
        }),
      ],
      refused: [
        {
          path: "F:/films/notes.txt",
          error: { code: "INVALID_INPUT", details: [{ key: "VIDEO_ALREADY_LISTED" }] },
        },
      ],
    });
    show();

    fireEvent.click(await screen.findByRole("button", { name: ru.ui.video.add }));

    await waitFor(() =>
      expect(mockVideoAdd).toHaveBeenCalledWith(
        "srv_1",
        ["F:/films/a.mkv", "F:/films/notes.txt"],
        null,
      ),
    );
    const a = await card("a");
    expect(a.getByText(ru.ui.video.planning)).toBeInTheDocument();
    const refused = await screen.findByTestId("refused");
    expect(refused).toHaveTextContent("notes.txt");
    expect(refused).toHaveTextContent(ru.details.VIDEO_ALREADY_LISTED);
  });

  it("a plan waiting for a place among the heavy work says it is queued (T688)", async () => {
    mockVideoList.mockResolvedValue([
      video({
        id: "a",
        state: "planning",
        plan: null,
        progress: {
          task_state: "queued",
          progress: 0,
          speed_bps: null,
          eta_s: null,
          rung: null,
          rungs: 0,
        },
      }),
      video({ id: "b", state: "planning", plan: null }),
    ]);
    show();
    const a = await card("a");
    expect(a.getByRole("status")).toHaveTextContent(
      `${ru.ui.video.planning} · ${ru.ui.video.queued}`,
    );
    expect((await card("b")).getByRole("status")).toHaveTextContent(ru.ui.video.planning);
    expect((await card("b")).getByRole("status")).not.toHaveTextContent(ru.ui.video.queued);
  });

  it("opens the file dialog by itself when the library sent somebody here to add", async () => {
    mockOpen.mockResolvedValue(["F:/films/a.mkv"]);
    mockVideoAdd.mockResolvedValue({ added: [video({ id: "a" })], refused: [] });
    show("ru", "/video?add=1");

    await waitFor(() => expect(mockOpen).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(mockVideoAdd).toHaveBeenCalledWith("srv_1", ["F:/films/a.mkv"], null),
    );
  });

  it("builds a set into the medium the library sent here: one file, with its id (T675)", async () => {
    mockOpen.mockResolvedValue("F:/films/Фильм.mkv");
    mockVideoAdd.mockResolvedValue({
      added: [video({ id: "a", media_id: "m1", state: "planning", plan: null })],
      refused: [],
    });
    show("ru", "/video?media=m1");

    await waitFor(() => expect(mockOpen).toHaveBeenCalledTimes(1));
    expect(mockOpen.mock.calls[0][0]).toMatchObject({ multiple: false });
    await waitFor(() =>
      expect(mockVideoAdd).toHaveBeenCalledWith("srv_1", ["F:/films/Фильм.mkv"], "m1"),
    );
    // It is on this screen with its plan coming, like any other video.
    expect((await card("a")).getByText(ru.ui.video.planning)).toBeInTheDocument();
  });

  it("says why a medium would not take a set, and adds nothing (T675)", async () => {
    mockOpen.mockResolvedValue("F:/films/Фильм.mkv");
    mockVideoAdd.mockRejectedValue({ code: "MEDIA_HAS_SET", details: [], cause: null });
    show("ru", "/video?media=m1");
    expect(await screen.findByText(ru.errors.MEDIA_HAS_SET.message)).toBeInTheDocument();
    expect(screen.queryByTestId(/^video-/)).toBeNull();
  });

  it("adds a medium's video stopped on a set nobody owns, with «Replace» (T677)", async () => {
    mockOpen.mockResolvedValue("F:/films/Фильм.mkv");
    const held = video({
      id: "a",
      media_id: "m1",
      state: "problem",
      problem: {
        error: {
          code: "MEDIA_HAS_SET",
          details: [{ key: "OLD_SET_UNRECOGNIZED", params: { name: "film" } }],
          cause: "film",
        },
        actions: ["replace"],
      },
    });
    mockVideoAdd.mockResolvedValue({ added: [held], refused: [] });
    mockVideoReplace.mockResolvedValue({ ...held, state: "working", problem: null });
    show("ru", "/video?media=m1");

    const c = await card("a");
    // Added, not refused: the card is there with its one-line problem and «Replace» only.
    expect(c.getByRole("alert")).toHaveTextContent(ru.errors.MEDIA_HAS_SET.message);
    expect(c.queryByRole("button", { name: ru.ui.video.retry })).toBeNull();
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.replace }));
    const ask = c.getByRole("group", { name: ru.ui.video.replace });
    expect(ask).toHaveTextContent("Старый набор «Фильм» будет удалён и собран заново");
    expect(mockVideoReplace).not.toHaveBeenCalled();
    fireEvent.click(within(ask).getByRole("button", { name: ru.ui.video.replace }));
    await waitFor(() => expect(mockVideoReplace).toHaveBeenCalledWith("a", false));
  });

  it("says there is no server rather than offering to add", async () => {
    useServers.setState({ profiles: [], loading: false });
    mockServersList.mockResolvedValue([]);
    show();
    expect(await screen.findByText(ru.ui.video.noServer, { exact: false })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: ru.ui.video.add })).toBeDisabled();
  });
});

describe("the plan before Start", () => {
  it("says what the time covers and when it is only an estimate (T689)", async () => {
    mockVideoList.mockResolvedValue([
      video({ id: "a" }),
      video({
        id: "b",
        plan: plan({
          from: "formula",
          needs_measuring: true,
          measure_s: 600,
          encode_s: 1200,
          encode_estimate: "model",
        }),
      }),
    ]);
    show();
    const measured = (await card("a")).getByTestId("plan");
    expect(measured).toHaveTextContent(`${ru.ui.video.encodeTime} ≈ 25 мин`);
    expect(measured).not.toHaveTextContent(ru.ui.video.preliminary);
    const guessed = (await card("b")).getByTestId("plan");
    expect(guessed).toHaveTextContent(`${ru.ui.video.measureAndEncodeTime} ≈ 30 мин`);
    expect(guessed).toHaveTextContent(ru.ui.video.preliminary);
  });

  it("lists the rungs, the size on the server and the time", async () => {
    mockVideoList.mockResolvedValue([video()]);
    show();
    const c = await card();
    const p = c.getByTestId("plan");
    expect(p).toHaveTextContent("1080p · 8 Мбит/с");
    expect(p).toHaveTextContent("720p · 4 Мбит/с");
    expect(p).toHaveTextContent("На сервере ≈ 6,0 ГБ");
    expect(p).toHaveTextContent("≈ 25 мин");
    expect(c.queryByRole("list", { name: ru.ui.video.stagesLabel })).toBeNull();
  });

  it("says in one line what is short and whether the name is taken", async () => {
    mockVideoList.mockResolvedValue([
      video({
        plan: plan({
          server_space: {
            state: "short",
            needed_bytes: 10,
            free_bytes: 5,
            short_by: 2 * 1024 ** 3,
          },
          local_space: { state: "short", needed_bytes: 10, free_bytes: 5, short_by: 1024 ** 3 },
          name_taken: true,
        }),
      }),
    ]);
    show();
    const p = (await card()).getByTestId("plan");
    expect(p).toHaveTextContent("Не хватает 2,0 ГБ на сервере");
    expect(p).toHaveTextContent("Не хватает 1,0 ГБ на этом компьютере");
    expect(p).toHaveTextContent("Имя «film» занято");
  });

  it("offers a choice of audio only when there is more than one track", async () => {
    mockVideoList.mockResolvedValue([video({ source: source(2) }), video({ id: "v2" })]);
    show();
    const c = await card("v1");
    const select = c.getByRole("combobox");
    fireEvent.change(select, { target: { value: "1" } });
    await waitFor(() => expect(mockVideoSetAudio).toHaveBeenCalledWith("v1", 1));
    expect((await card("v2")).queryByRole("combobox")).toBeNull();
  });

  it("starts one video, and «Start all» starts every ready one", async () => {
    mockVideoList.mockResolvedValue([video({ id: "a" }), video({ id: "b" }), working({ id: "c" })]);
    show();
    fireEvent.click((await card("a")).getByRole("button", { name: ru.ui.video.start }));
    await waitFor(() => expect(mockVideoStart).toHaveBeenCalledWith(["a"]));

    fireEvent.click(screen.getByRole("button", { name: ru.ui.video.startAll }));
    await waitFor(() => expect(mockVideoStart).toHaveBeenLastCalledWith(["a", "b"]));
  });

  it("shows a start refused for one video on that video's card", async () => {
    mockVideoList.mockResolvedValue([video()]);
    mockVideoStart.mockResolvedValue([{ id: "v1", error: { code: "VIDEO_NOT_NOW" } }]);
    show();
    fireEvent.click((await card()).getByRole("button", { name: ru.ui.video.start }));
    expect(await (await card()).findByText(ru.errors.VIDEO_NOT_NOW.message)).toBeInTheDocument();
  });

  it("lets the title be changed while there is no medium yet", async () => {
    mockVideoList.mockResolvedValue([video(), working({ id: "w" })]);
    show();
    const c = await card();
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.editTitle }));
    fireEvent.change(c.getByLabelText(ru.ui.video.title), { target: { value: "Новое" } });
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.saveTitle }));
    await waitFor(() => expect(mockVideoSetName).toHaveBeenCalledWith("v1", "Новое", null));
    expect((await card("w")).queryByRole("button", { name: ru.ui.video.editTitle })).toBeNull();
  });

  it("takes a video off the list with «Remove»", async () => {
    mockVideoList.mockResolvedValue([video()]);
    show();
    fireEvent.click((await card()).getByRole("button", { name: ru.ui.video.remove }));
    await waitFor(() => expect(mockVideoRemove).toHaveBeenCalledWith("v1"));
    await waitFor(() => expect(screen.queryByTestId("video-v1")).toBeNull());
  });

  it("removing a video at work keeps the card, stopping, until the core says it is gone (T683)", async () => {
    mockVideoList.mockResolvedValue([working({ state: "paused", paused_by_person: true })]);
    mockVideoRemove.mockResolvedValue(
      working({ state: "cancelling", updated_at: "2026-10-01T10:05:00Z" }),
    );
    show();
    fireEvent.click((await card()).getByRole("button", { name: ru.ui.video.remove }));
    await waitFor(() => expect(mockVideoRemove).toHaveBeenCalledWith("v1"));
    const c = await card();
    await waitFor(() => expect(c.getByText(ru.ui.video.stopping)).toBeInTheDocument());
    expect(c.queryAllByRole("button")).toHaveLength(0);

    await waitFor(() => expect(pushRemoved).not.toBeNull());
    act(() => pushRemoved!("v1"));
    await waitFor(() => expect(screen.queryByTestId("video-v1")).toBeNull());
  });
});

describe("the stages after Start", () => {
  it("lights the current stage with its percentage, time left and rung k of n", async () => {
    mockVideoList.mockResolvedValue([working()]);
    show();
    const facts = (await card()).getByTestId("stage-facts");
    expect(facts).toHaveTextContent("42%");
    expect(facts).toHaveTextContent("осталось 10:00");
    expect(facts).toHaveTextContent("ступень 2 из 3");
  });

  it("the check shows no made-up number; the cutting shows its own share (T689)", async () => {
    const at = (stage: VideoView["stage"], progress: number) =>
      working({
        id: stage,
        stage,
        progress: {
          task_state: "running",
          progress,
          speed_bps: null,
          eta_s: null,
          rung: null,
          rungs: 4,
        },
      });
    mockVideoList.mockResolvedValue([at("verifying", 0), at("cutting", 0.25)]);
    show();
    const checking = await card("verifying");
    // No figure for a stage that cannot say how far it is: a bar without a value.
    expect(checking.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
    expect(checking.queryByTestId("stage-facts")?.textContent ?? "").not.toContain("%");
    const cutting = await card("cutting");
    expect(cutting.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "25");
    expect(cutting.getByTestId("stage-facts")).toHaveTextContent("25%");
  });

  it("offers pause while working and resume while paused, and cancel for both", async () => {
    mockVideoList.mockResolvedValue([working({ id: "a" }), working({ id: "b", state: "paused" })]);
    show();
    const a = await card("a");
    expect(a.queryByRole("button", { name: ru.ui.video.resume })).toBeNull();
    // «Remove» is there while working too (T683): the core stops the work first.
    fireEvent.click(a.getByRole("button", { name: ru.ui.video.pause }));
    await waitFor(() => expect(mockVideoPause).toHaveBeenCalledWith("a"));

    const b = await card("b");
    fireEvent.click(b.getByRole("button", { name: ru.ui.video.cancel }));
    await waitFor(() => expect(mockVideoCancel).toHaveBeenCalledWith("b"));
  });

  it("resumes a paused video", async () => {
    mockVideoList.mockResolvedValue([working({ id: "b", state: "paused" })]);
    show();
    const b = await card("b");
    fireEvent.click(b.getByRole("button", { name: ru.ui.video.resume }));
    await waitFor(() => expect(mockVideoResume).toHaveBeenCalledWith("b"));
  });

  it("offers «Continue» on the card after a pause pressed in «Tasks» (T685)", async () => {
    mockVideoList.mockResolvedValue([working()]);
    show();
    await card();
    await waitFor(() => expect(push).not.toBeNull());
    // The core makes the task's pause the video's own and says so.
    act(() =>
      push!(
        working({
          state: "paused",
          paused_by_person: true,
          updated_at: "2026-10-01T10:05:00Z",
          rev: 5,
          progress: { ...working().progress!, task_state: "paused" },
        }),
      ),
    );
    const c = await card();
    await waitFor(() =>
      expect(c.getByRole("button", { name: ru.ui.video.resume })).toBeInTheDocument(),
    );
    expect(c.queryByRole("button", { name: ru.ui.video.pause })).toBeNull();
    expect(c.getByText(ru.ui.video.paused)).toBeInTheDocument();
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.resume }));
    await waitFor(() => expect(mockVideoResume).toHaveBeenCalledWith("v1"));
  });

  it("says it is stopping while cancelling, and offers nothing to press", async () => {
    mockVideoList.mockResolvedValue([working({ state: "cancelling" })]);
    show();
    const c = await card();
    expect(c.getByText(ru.ui.video.stopping)).toBeInTheDocument();
    expect(c.queryAllByRole("button")).toHaveLength(0);
  });

  it("ends with the link and a button that copies it", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    mockVideoList.mockResolvedValue([
      video({
        stage: "done",
        state: "done",
        media_id: "m1",
        link: { origin: "https://s/x/master.m3u8", cdn: null },
      }),
    ]);
    show();
    const c = await card();
    expect(
      within(c.getByRole("list", { name: ru.ui.video.stagesLabel })).getByText(/Готово/),
    ).toHaveAttribute("data-mark", "passed");
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.copy }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("https://s/x/master.m3u8"));
    expect(await c.findByText(ru.ui.video.copied)).toBeInTheDocument();
  });
});

describe("a problem", () => {
  it("is one line, with the rest folded under «Details»", async () => {
    mockVideoList.mockResolvedValue([problem("SSH_UNREACHABLE", ["retry"])]);
    show();
    const alert = (await card()).getByRole("alert");
    expect(alert).toHaveTextContent(ru.errors.SSH_UNREACHABLE.message);
    const details = alert.querySelector("details");
    expect(details).not.toBeNull();
    expect(details!.open).toBe(false);
    expect(details).toHaveTextContent("ssh: broken pipe");
  });

  it("retries, builds anyway and replaces through the buttons the core offered", async () => {
    mockVideoList.mockResolvedValue([
      problem("FILE_IN_USE", ["build_anyway", "retry"]),
      { ...problem("SLUG_TAKEN", ["replace", "rename"]), id: "v2" },
    ]);
    // The core answers with the video still stopped, so both buttons stay to be pressed.
    mockVideoRetry.mockImplementation(() =>
      Promise.resolve(problem("FILE_IN_USE", ["build_anyway", "retry"])),
    );
    show();
    const c = await card();
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.buildAnyway }));
    await waitFor(() => expect(mockVideoRetry).toHaveBeenCalledWith("v1", true));
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.retry }));
    await waitFor(() => expect(mockVideoRetry).toHaveBeenCalledWith("v1", false));

    fireEvent.click((await card("v2")).getByRole("button", { name: ru.ui.video.replace }));
    // T676 — asked first, in one line; nothing is sent until it is answered.
    const ask = (await card("v2")).getByRole("group", { name: ru.ui.video.replace });
    expect(ask).toHaveTextContent("Старый набор «Фильм» будет удалён и собран заново");
    expect(mockVideoReplace).not.toHaveBeenCalled();
    fireEvent.click(within(ask).getByRole("button", { name: ru.ui.video.replace }));
    await waitFor(() => expect(mockVideoReplace).toHaveBeenCalledWith("v2", false));
  });

  it("asks «replace anyway» when somebody is watching, and «cancel» sends nothing", async () => {
    mockVideoList.mockResolvedValue([problem("SLUG_TAKEN", ["replace", "rename"])]);
    mockVideoReplace.mockImplementation(() =>
      Promise.resolve(problem("SLUG_TAKEN", ["replace", "rename"])),
    );
    mockVideoReplace.mockRejectedValueOnce({ code: "FILE_IN_USE", details: [], cause: null });
    show();
    const c = await card();
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.replace }));
    fireEvent.click(
      within(c.getByRole("group", { name: ru.ui.video.replace })).getByRole("button", {
        name: ru.ui.video.replace,
      }),
    );
    await waitFor(() => expect(mockVideoReplace).toHaveBeenCalledWith("v1", false));
    fireEvent.click(await c.findByRole("button", { name: ru.ui.video.replaceAnyway }));
    await waitFor(() => expect(mockVideoReplace).toHaveBeenCalledWith("v1", true));

    // Asked again and let go: nothing more is sent, the problem's buttons are back.
    mockVideoReplace.mockClear();
    fireEvent.click(await c.findByRole("button", { name: ru.ui.video.replace }));
    fireEvent.click(c.getByRole("button", { name: ru.ui.common.cancel }));
    expect(c.queryByRole("group", { name: ru.ui.video.replace })).toBeNull();
    expect(mockVideoReplace).not.toHaveBeenCalled();
  });

  it("asks for another name and goes on with it", async () => {
    mockVideoList.mockResolvedValue([problem("SLUG_TAKEN", ["replace", "rename"])]);
    show();
    const c = await card();
    // «Rename» is offered as the field and its «Retry».
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.retry }));
    fireEvent.change(c.getByLabelText(ru.ui.video.title), { target: { value: "Фильм 2" } });
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.retry }));
    await waitFor(() => expect(mockVideoSetName).toHaveBeenCalledWith("v1", "Фильм 2", null));
    await waitFor(() => expect(mockVideoRetry).toHaveBeenCalledWith("v1", false));
  });

  it("opens the rung editor here, saves the rungs to the video and carries on", async () => {
    mockVideoList.mockResolvedValue([problem("LADDER_OBJECTION", ["build_anyway", "edit_rungs"])]);
    mockVideoSetRungs.mockImplementation(() =>
      Promise.resolve(problem("LADDER_OBJECTION", ["build_anyway", "edit_rungs"])),
    );
    show();
    const c = await card();
    fireEvent.click(c.getByRole("button", { name: ru.ui.video.rungs }));
    const editor = await screen.findByRole("dialog", { name: ru.ui.video.rungs });
    // Leave the lighter rung out, then save.
    fireEvent.click(within(editor).getAllByRole("checkbox")[1]);
    fireEvent.click(within(editor).getByRole("button", { name: ru.ui.video.saveRungs }));

    await waitFor(() => expect(mockVideoSetRungs).toHaveBeenCalledTimes(1));
    const [id, sent] = mockVideoSetRungs.mock.calls[0] as [string, Rung[]];
    expect(id).toBe("v1");
    expect(sent.map((r) => r.bitrate_bps)).toEqual([8_000_000]);
    await waitFor(() => expect(mockVideoRetry).toHaveBeenCalledWith("v1", false));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("the rungs before Start", () => {
  it("a rung retyped by hand is saved without an error and is marked to measure (T680)", async () => {
    mockVideoList.mockResolvedValue([video()]);
    const unmeasured = {
      ...rung(1, 3, 720),
      reasons: ["edited_by_hand"],
      quality: { state: "not_measured" },
    } as Rung;
    mockLadderRecomputeRung.mockResolvedValue(unmeasured);
    mockVideoSetRungs.mockImplementation(() =>
      Promise.resolve(
        video({
          plan: plan({
            rungs: [rung(0, 8, 1080), unmeasured],
            from: "edited",
            needs_measuring: true,
            measure_s: 120,
          }),
        }),
      ),
    );
    show();
    fireEvent.click((await card()).getByRole("button", { name: ru.ui.video.rungs }));
    const editor = await screen.findByRole("dialog", { name: ru.ui.video.rungs });
    fireEvent.change(within(editor).getByLabelText(`${ru.ui.ladder.columnBitrate} 2`), {
      target: { value: "3" },
    });
    await waitFor(() => expect(mockLadderRecomputeRung).toHaveBeenCalled());
    await waitFor(() =>
      expect(within(editor).getByTestId("rung-1")).toHaveTextContent(ru.ui.ladder.notMeasured),
    );
    fireEvent.click(within(editor).getByRole("button", { name: ru.ui.video.saveRungs }));

    await waitFor(() => expect(mockVideoSetRungs).toHaveBeenCalledTimes(1));
    const [, sent] = mockVideoSetRungs.mock.calls[0] as [string, Rung[]];
    expect(sent[1].quality).toEqual({ state: "not_measured" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
    expect(mockVideoStart).not.toHaveBeenCalled();
    // The plan says which rung is still to be measured.
    const p = (await card()).getByTestId("plan");
    expect(p).toHaveTextContent(`720p · 3 Мбит/с · ${ru.ui.video.rungToMeasure}`);
    expect(p).not.toHaveTextContent(`1080p · 8 Мбит/с · ${ru.ui.video.rungToMeasure}`);
  });

  it("are edited on this screen and saved to the video without starting it", async () => {
    mockVideoList.mockResolvedValue([video()]);
    show();
    fireEvent.click((await card()).getByRole("button", { name: ru.ui.video.rungs }));
    const editor = await screen.findByRole("dialog", { name: ru.ui.video.rungs });
    fireEvent.click(within(editor).getByRole("button", { name: ru.ui.video.resetRungs }));
    await waitFor(() => expect(mockVideoSetRungs).toHaveBeenCalledWith("v1", null));
    expect(mockVideoRetry).not.toHaveBeenCalled();
    expect(mockVideoStart).not.toHaveBeenCalled();
  });
});

describe("both languages", () => {
  it("speaks English when asked to", async () => {
    mockVideoList.mockResolvedValue([working()]);
    show("en");
    expect(await screen.findByRole("heading", { name: en.ui.video.heading })).toBeInTheDocument();
    expect((await card()).getByTestId("stage-facts")).toHaveTextContent("rung 2 of 3");
  });
});
