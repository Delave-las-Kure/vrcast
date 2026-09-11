/**
 * T199, T241 — the quality set for one film.
 *
 * The screen answers three questions in order, and the order is the point:
 *
 *   1. what the source actually is — the average and, more importantly, the **peak**, which
 *      is what a viewer's connection has to hold (FR-040);
 *   2. where these rungs came from — measured here, borrowed, or a guess from the formula.
 *      A person is never left to assume (FR-145, R-21);
 *   3. what each rung is worth, and what is wrong with the set.
 *
 * **Building is offered only when there is a measurement** (FR-141). Until then the button
 * says why it cannot be pressed rather than being quietly absent — a control that vanishes
 * teaches people that the application is broken.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useSearchParams } from "react-router-dom";

import { ErrorNotice } from "../shared/ErrorNotice";
import { Borrow } from "./Borrow";
import { MeasuredPoints } from "./MeasuredPoints";
import { slugOf } from "../shared/names";
import { RungEditor } from "./RungEditor";
import { useActiveServer } from "../servers/store";
import { useLang, useT } from "../../shared/i18n";
import { fill, renderDetail } from "../../shared/i18n/render";
import { ipc, onTaskDone } from "../../shared/ipc";
import type {
  AppError,
  LadderPreview,
  MachineSpeed,
  MeasurePreview,
  MediaView,
  Detail,
  Rung,
  SourceMeasured,
} from "../../shared/contract";

function bitrate(bps: number): string {
  return `${(bps / 1_000_000).toFixed(1)} Mbit/s`;
}

/** Where the rungs came from, said plainly. */
function Provenance({ preview }: { preview: LadderPreview }) {
  const t = useT();
  const words = t.ui.ladder;

  if (preview.from === "measured") {
    return <p data-testid="provenance">{words.fromMeasured}</p>;
  }
  if (preview.from === "borrowed") {
    return <p data-testid="provenance">{words.fromBorrowed}</p>;
  }
  return (
    <div data-testid="provenance" role="note">
      <p>{words.fromFormula}</p>
      <p>{words.formulaExplain}</p>
    </div>
  );
}

/** What measuring would cost, before anybody agrees to it (FR-147). */
/** Where the estimate came from, said in the language in use. */
function estimateFrom(
  machine: MachineSpeed,
  words: { estimateFromThisMachine: string; estimateFromModel: string; estimateNotAsked: string },
): string {
  if (machine.state === "nothing_timed_yet") return words.estimateFromModel;
  if (machine.state === "not_asked") return words.estimateNotAsked;
  return words.estimateFromThisMachine
    .replace("{points}", String(machine.points))
    .replace("{seconds}", (machine.seconds_per_point_x10 / 10).toFixed(1))
    .replace("{times}", (machine.factor_x100 / 100).toFixed(1));
}

function MeasureOffer({
  preview,
  onStart,
  running,
}: {
  preview: MeasurePreview;
  onStart: () => void;
  running: boolean;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.ladder;
  const left = Math.max(0, preview.points - preview.already_measured);
  // **The floor of one minute is for a short job, not for no job** (T425). `Math.max(1, …)`
  // is right while there is something left to do — "about 0 minutes" reads as a mistake —
  // and wrong when there is nothing: it offers a minute of work on a grid that is already
  // measured, and a person who presses the button gets an immediate finish and no idea why.
  const minutes = Math.max(1, Math.round(preview.about_seconds / 60));
  const nothingLeft = left === 0;

  return (
    <section aria-label={words.measureTitle}>
      <h3>{words.measureTitle}</h3>
      <p>{words.measureExplain}</p>
      <p data-testid="how-long">
        {nothingLeft
          ? words.measureNothingLeft
          : preview.already_measured > 0
            ? words.measureTakesResume
                .replace("{minutes}", String(minutes))
                .replace("{points}", String(left))
                .replace("{total}", String(preview.points))
            : words.measureTakes
                .replace("{minutes}", String(minutes))
                .replace("{points}", String(preview.points))}
      </p>
      {/*
        Whose machine the estimate is from. The difference between twenty minutes and two
        hours is the whole decision, and an estimate taken on somebody else's machine has no
        business pretending otherwise.

        Three answers, not two (T423, T424). "From the model because you have no timings" and
        "from the model because your timings could not be read" look the same to a person and
        are not the same thing at all: the second may be wrong by a factor of three, and only
        saying so gives anybody a reason to doubt the number.
      */}
      <p data-testid="estimate-from">{estimateFrom(preview.machine, words)}</p>
      {/* What the core worked out about this material and had nowhere to say (T422). The
          probe can fail, or run on a card it was not calibrated for, and either makes the
          top of the grid a different kind of number — which is worth knowing *before*
          agreeing to an hour of encoding, not after. `preview.notices` was carried across
          the boundary and never read. */}
      {preview.notices.length > 0 && (
        <ul data-testid="offer-notices">
          {preview.notices.map((notice, i) => (
            <li key={i} role="note">
              {renderDetail(notice, t, lang)}
            </li>
          ))}
        </ul>
      )}

      {/* Folded away: wanted by whoever doubts the estimate, and by nobody else. */}
      <details data-testid="stands-on">
        <summary>{words.measureStandsOn}</summary>
        <p>
          {fill(
            words.measureChunks,
            { starts: preview.chunk_starts.map((at) => Math.round(at / 60)).join(", ") },
            t,
            lang,
          )}
        </p>
        <p>{fill(words.measureAnchor, { mbps: preview.anchor_mbps }, t, lang)}</p>
      </details>

      <button type="button" onClick={onStart} disabled={running || nothingLeft}>
        {words.measureStart}
      </button>
      {running && <p role="status">{words.measureRunning}</p>}
    </section>
  );
}

/**
 * The screen with its own file to choose.
 *
 * Separate from [`LadderScreen`] so that the screen itself can be checked without a file
 * dialogue — the dialogue belongs to the system and cannot be opened in a test.
 */
export function LadderPage() {
  const t = useT();
  const words = t.ui.ladder;
  // The file the preparation screen just made, if that is where this came from. In the
  // address rather than in a store, the way the servers section already hands a server to
  // the deployment screen: it survives a reload, it can be linked to, and it needs no state
  // shared between two screens that otherwise know nothing about each other.
  const [params] = useSearchParams();
  const handed = params.get("file");
  const [picked, setPicked] = useState<string | null>(null);
  const path = picked ?? handed;
  const server = useActiveServer();

  const pick = async () => {
    const chosen = await open({
      multiple: false,
      directory: false,
      filters: [
        {
          name: "video",
          extensions: ["mp4", "mkv", "mov", "webm", "m4v", "avi", "ts"],
        },
      ],
    });
    if (typeof chosen === "string") setPicked(chosen);
  };

  if (!path) {
    return (
      <div>
        <h2>{words.rungs}</h2>
        <p>{words.explain}</p>
        <button type="button" onClick={pick}>
          {words.pickFile}
        </button>
      </div>
    );
  }
  return (
    <div>
      {handed && !picked && (
        <p className="muted" data-testid="handed-from">
          {words.handedFrom.replace("{path}", handed)}{" "}
          <button type="button" className="button-link" onClick={() => void pick()}>
            {words.pickAnother}
          </button>
        </p>
      )}
      <LadderScreen path={path} serverId={server?.id ?? null} slug={slugOf(path)} />
    </div>
  );
}

export function LadderScreen({
  path,
  serverId,
  slug,
}: {
  path: string;
  /** Which server the set is built on. Nothing can be built without one. */
  serverId?: string | null;
  /** The medium's own directory on that server. */
  slug?: string;
}) {
  const t = useT();
  const words = t.ui.ladder;

  const { lang } = useLang();
  const [source, setSource] = useState<SourceMeasured | null>(null);
  const [preview, setPreview] = useState<LadderPreview | null>(null);
  const [offer, setOffer] = useState<MeasurePreview | null>(null);
  const [rungs, setRungs] = useState<Rung[]>([]);
  // Whether the core is being asked for a ladder right now.
  //
  // `ladder_plan` runs the complexity probe — three encodes — and reads the database. On a
  // feature film that is seconds, and the screen used to spend them showing a heading and
  // nothing else, which reads as broken (FR-145 says where the rungs came from must always
  // be plain; saying nothing at all is not plain).
  const [working, setWorking] = useState(true);
  // Which measurement is running, if any.
  //
  // **In a ref, not in state, and that is the whole of the fix.** The screen has to know
  // *which* task ended — a person may have a preparation and a transfer running beside
  // this. It used to be state, and the `task:done` handler cleared it first thing; that is
  // the listening effect's own dependency, so React tore the effect down, its cleanup set
  // the local `alive` flag to false, and the measured rungs the core sent a moment later
  // were dropped by `if (!alive) return;`. The measurement finished, was written down, and
  // the screen went on showing the guess. Found on 2026-08-28 from an owner's report.
  const measuringId = useRef<string | null>(null);
  /** What the measurement had to say about itself. Cleared when the file changes. */
  const [measured, setMeasured] = useState<Detail[]>([]);
  const [measuring, setMeasuring] = useState(false);
  // T587 — a per-file generation token, the same pattern `uploadGenRef` already uses in
  // `UploadScreen` (T582/T585). This used to be a plain boolean (`alive`): true while the
  // screen was open, false once torn down. That answers "is the component still mounted",
  // not "is this particular answer still about the file on screen right now" — and because
  // `LadderPage` renders this same component instance across a file change (no `key`, so no
  // remount), the effect for the NEW file flips the very same boolean back to true before an
  // in-flight request for the OLD file has any chance to come back. A `loadPlan`/
  // `ladderMeasure` response for file A, arriving after file B is already open, then read
  // `alive.current === true` — correctly true, just about the wrong file — and overwrote B's
  // screen with A's source, rungs and provenance. Bumped once per file change; every
  // callback below captures its own value at request-start and compares it at
  // response-time, so an answer is accepted only for the file that actually asked it.
  const genRef = useRef(0);
  const [name, setName] = useState(slug ?? "");
  // The most recent answer, read by `loadPlan` itself rather than by adding `preview` to its
  // own dependency list (T522). `preview` is *set* by `loadPlan`, so making the callback
  // depend on it would give it a new identity every time it succeeds — and every effect that
  // depends on `loadPlan` (there are two below) would fire again, calling it again, setting a
  // new `preview` object again, forever. A ref sidesteps that: it is always current when
  // `loadPlan` runs, and updating it changes nothing about the callback's own identity.
  const previewRef = useRef<LadderPreview | null>(null);
  // The peak `ladderMeasure` last found for this file, read by `loadPlan` the same way as
  // `previewRef` above and for the same reason (T522): `loadPlan` must not depend on it, or
  // the reload the measurement's own handler triggers below would give `loadPlan` a new
  // identity, re-running the effects that depend on it, without end. Reset alongside
  // `previewRef` when the file itself changes — a past file's peak has nothing to say about
  // this one's ladder.
  const measuredPeakRef = useRef<number | null>(null);
  // T522 — the two fields the core already reads off `LadderRequest` and the screen never
  // gave anyone a way to fill in. Kept as strings on screen and turned into the request's
  // shape only when they hold something: `native_height` is an `Option<u32>` in the core,
  // and `declared_layout` an `Option<Layout>` — leaving either blank must mean "unknown",
  // not "flat" or "not stretched", which are both claims about the file, not silence.
  const [nativeHeightInput, setNativeHeightInput] = useState("");
  const [declaredLayout, setDeclaredLayout] = useState<
    "" | "Flat" | "SideBySide" | "OverUnder"
  >("");
  // Which rungs the person has left out. By the rung's own index rather than by position,
  // so that editing a bitrate — which rebuilds the array — does not silently move the
  // choice onto a different rung.
  const [leftOut, setLeftOut] = useState<ReadonlySet<number>>(new Set());
  const [building, setBuilding] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  // T571, T574 — the core refuses `ladder_build` with `FILE_IN_USE` when somebody is
  // watching the set right now and `confirmed` was not sent. The refusal names nothing
  // to renew (fixed wording, like `media_rename`'s own FILE_IN_USE, T545), so there is
  // nothing to compose — only a second button that retries with `confirmed: true`.
  const [buildFileInUse, setBuildFileInUse] = useState(false);
  // T528 — media already on this server's library, offered explicitly instead of leaving
  // the link between "this set" and "that medium" to a guessed slug (`slugOf(path)`)
  // matching by accident. Empty while `serverId` is unknown: there is nothing to list.
  const [existingMedia, setExistingMedia] = useState<MediaView[]>([]);
  // "" means "new set" — the field below works exactly as it always did, driven by
  // `name`. Anything else names a medium in `existingMedia` whose `slug` is sent as-is.
  const [selectedMediaId, setSelectedMediaId] = useState<string>("");
  const selectedMedia = existingMedia.find((m) => m.id === selectedMediaId) ?? null;

  useEffect(() => {
    // T587 — invalidates every request in flight for whatever file was open before this
    // one, in every callback that captures `genRef.current` at its own start. Bumped here
    // and nowhere else: one point where "the file changed" is decided, rather than a
    // cleanup racing the next effect's own setup (see the note on `genRef` above).
    genRef.current += 1;
    // A different file was never measured under any codec, so there is nothing yet to ask
    // a repeat `ladderPlan` call to keep asking under (see `loadPlan` below) — reset only
    // when the file itself changes, not on every edit of the advanced fields, or a codec
    // this file was measured under would be forgotten the moment somebody typed a height.
    previewRef.current = null;
    measuredPeakRef.current = null;
    // T574 — a refusal on a past file has nothing to say about this one: a fresh path is
    // not necessarily still in use by anyone.
    setBuildFileInUse(false);
    // T587 — a measurement task id from a past file has nothing to say about this one
    // either: without this, `onTaskDone` below (subscribed fresh for the new file, but
    // reading the same ref) could still match a task started on the file just left, and
    // paint this screen with that other file's notices the moment it finishes.
    measuringId.current = null;
  }, [path]);

  // T528 — the library of the server this set would be built on, fetched only when a
  // server is actually known: without one there is nothing to list, and asking would be
  // asking about a server nobody picked.
  useEffect(() => {
    if (!serverId) {
      setExistingMedia([]);
      return;
    }
    let alive2 = true;
    ipc
      .libraryList(serverId)
      .then((view) => {
        if (alive2) setExistingMedia(Array.isArray(view?.media) ? view.media : []);
      })
      .catch(() => undefined);
    return () => {
      alive2 = false;
    };
  }, [serverId]);

  /**
   * Ask the core for the ladder and put its answer on screen.
   *
   * One function for both the first load and the reload after a measurement. It used to be
   * written out twice, in two effects, and the two could drift — one of them already had a
   * line the other did not.
   */
  const loadPlan = useCallback(async (): Promise<LadderPreview | null> => {
    // T587 — captured before the request goes out, compared once it comes back. A stale
    // answer (this file's generation has since moved on) is dropped in every branch
    // below — success, failure and `finally` alike — none of the three gets to touch a
    // screen that has already moved on to another file.
    const gen = genRef.current;
    setWorking(true);
    try {
      // T522 — the two "Advanced" fields, sent along on every call. Blank means unknown to
      // the core, not "flat" or "not stretched": `native_height` is left off rather than
      // sent as some default, and `declared_layout` likewise — the core's `Option` already
      // distinguishes "not told" from a value, and turning a blank field into a guess here
      // would be making up an answer nobody gave.
      const trimmedHeight = nativeHeightInput.trim();
      const nativeHeight = trimmedHeight === "" ? undefined : Number(trimmedHeight);
      const answer = await ipc.ladderPlan({
        path,
        // Consistent with `ConvertScreen`, which sends `prefer_hardware: true` without
        // asking (see the note at the top of that file): the encoder already decides for
        // itself whether hardware helps, and nothing here gives a person grounds to
        // second-guess it that `ConvertScreen` doesn't equally have.
        prefer_hardware: true,
        // `codec` asks for a plan for the codec a measurement was already made under — so
        // that measurement is not thrown away by asking for a different codec's plan by
        // accident. There is nothing to ask for on the very first call for a file: no
        // measurement has happened yet, so `previewRef` is still null and the field is
        // left off. The core then falls back to its own default (`h264`,
        // `#[serde(default = "h264")]`), which is right for a file nobody has measured
        // anything about yet.
        codec: previewRef.current?.codec,
        native_height:
          nativeHeight !== undefined && Number.isFinite(nativeHeight) ? nativeHeight : undefined,
        declared_layout: declaredLayout === "" ? undefined : declaredLayout,
        // The peak the last completed `ladderMeasure` found for this file, when there is
        // one (T522). Left off — not sent as `null` or `0` — until a measurement has
        // actually finished: the very first call for a freshly opened file always makes
        // this call before `ladderMeasure` can possibly have answered (it reads every
        // packet in the file, far slower than the complexity probe this call itself
        // uses), and sending nothing here is exactly what lets the core fall back to its
        // own probe-based anchor, which is the correct old behaviour for that moment.
        measured_peak_bps: measuredPeakRef.current ?? undefined,
      });
      if (gen !== genRef.current) return null;
      previewRef.current = answer;
      setPreview(answer);
      setRungs(answer.plan.rungs);
      if (answer.from !== "formula") setOffer(null);
      return answer;
    } catch (e) {
      if (gen === genRef.current) setError(e as AppError);
      return null;
    } finally {
      if (gen === genRef.current) setWorking(false);
    }
  }, [path, nativeHeightInput, declaredLayout]);

  useEffect(() => {
    if (!path) return;
    // T587 — this effect runs right after the one above bumped `genRef`, so this always
    // captures the generation the NEW file was just given, not the one it is leaving.
    const gen = genRef.current;
    setError(null);
    // A different file has not been measured, so nothing is known about its measurement.
    setMeasured([]);

    void loadPlan().then((answer) => {
      // The offer to measure is only worth fetching when there is nothing measured yet:
      // it runs the complexity probe, and running that on a screen that already has an
      // answer would cost seconds for nothing.
      if (gen !== genRef.current || answer?.from !== "formula") return;
      ipc
        .qualityMeasurePreview({ path })
        .then((o) => {
          if (gen === genRef.current) setOffer(o);
        })
        .catch(() => undefined);
    });

    // The source's own numbers, separately: measuring the peaks reads every packet and is
    // slower than planning, and there is no reason to make the rungs wait for it.
    ipc
      .ladderMeasure(path)
      .then((m) => {
        if (gen !== genRef.current) return;
        setSource(m);
        // T522 — the measured peak reaches the shown ladder only if it is actually sent
        // back to the core: the first `loadPlan()` above started before this measurement
        // could possibly have finished (see the comment on `measured_peak_bps` in
        // `loadPlan`), so it went out anchored on the complexity probe's guess. Asking
        // again, now that the real peak is known, is the only way it ever influences what
        // is on screen — leaving it in the ref for some later manual refresh would strand
        // it for a file that, in the ordinary case, nobody re-opens.
        measuredPeakRef.current = m.peak_bps;
        void loadPlan();
      })
      .catch(() => undefined);
  }, [path, loadPlan]);

  // **The end of the measurement reaches the screen.** Without this the task runs to its
  // end, the rungs it chose sit in the store, and this screen goes on saying "measuring"
  // until somebody thinks to close it and open it again.
  //
  // It listens for as long as the file is on screen, and reads which task is its own from
  // a ref. The subscription must not depend on what is running: the handler's first act is
  // to change that, and an effect that tears itself down mid-handler loses the answer it
  // was waiting for.
  useEffect(() => {
    if (!path) return;
    // T587 — kept in step with the other two call sites above, even though the
    // `measuringId.current = null` reset in the file-change effect already keeps a task
    // left running by a past file from matching by id in the ordinary case: cheap, and
    // correct if that invariant is ever loosened later without this comparison in mind.
    const gen = genRef.current;
    const unlisten = onTaskDone((event) => {
      if (event.id !== measuringId.current) return;
      measuringId.current = null;
      if (event.error) {
        setMeasuring(false);
        setError(event.error);
        return;
      }

      // Asked afresh rather than patched together here: the core decides what the rungs
      // are, and a screen that assembled its own would be a second opinion.
      //
      // "Measuring" stays on screen until the new rungs are actually there. Clearing it
      // first left a gap — sometimes a long one — in which the work was done, the screen
      // said nothing was happening, and the old guess was still on it.
      // What the measurement itself had to say (T416). A partial measurement — points that
      // would not encode — is not a failure and does not stop anything, but it is an
      // argument against building from what came out: where the points were missing, the
      // optimum may simply not have been found. So it is kept until the file changes and
      // shown beside the button it argues against, not as a passing message.
      // Tolerant of an event that carries none: the field is in the contract and the core
      // always sends it, but an aside going missing must not take the whole screen down
      // with it.
      if (gen === genRef.current) setMeasured(event.notices ?? []);
      void loadPlan().finally(() => {
        if (gen === genRef.current) setMeasuring(false);
      });
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [path, loadPlan]);

  const blocked = preview?.verdict.not_buildable ?? null;

  // T571, T574 — the core refuses `ladder_build` with `FILE_IN_USE` when the set is being
  // watched right now and `confirmed` was not sent along. One function for the plain
  // button and the "build anyway" retry, so the two cannot drift on what they send.
  const doBuild = (confirmed?: boolean) => {
    if (!preview || !serverId) return;
    setBuilding(true);
    ipc
      .ladderBuild({
        server_id: serverId,
        path,
        // T528 — an explicit choice from the library wins outright: its slug is
        // exactly what the medium already answers to, and sending anything else
        // would be the very guesswork this exists to remove. Only when nothing was
        // picked does the typed name (or, failing that, a guess from the file name)
        // apply, unchanged from before.
        slug: selectedMedia?.slug ?? (name.trim() || slugOf(path)),
        // Only what was asked for. The core names each variant by its own megabits
        // (`film_22.mp4`), not by its place in the list, so a gap in the numbering
        // costs nothing.
        rungs: rungs.filter((rung) => !leftOut.has(rung.index)),
        confirmed: confirmed ?? false,
      })
      .then(() => setBuildFileInUse(false))
      .catch((e: AppError) => {
        setError(e);
        setBuildFileInUse(e.code === "FILE_IN_USE");
      })
      .finally(() => setBuilding(false));
  };

  return (
    <div>
      <h2>{words.rungs}</h2>
      <p>{words.explain}</p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {/* T571, T574, T579 — the core refuses a build with `FILE_IN_USE` when the set is
          being watched right now and nobody confirmed going on regardless. Same pattern
          as `RenameMediaDialog`'s `renameAnyway` (T545): the refusal carries no numbers
          of its own to compose, only a choice to make. Placed right after the error it
          answers, a couple of Tab stops away, rather than after the whole form below —
          T579 moved this here from just below the main `build` button. */}
      {buildFileInUse && (
        <button
          type="button"
          data-testid="build-anyway"
          disabled={building}
          onClick={() => doBuild(true)}
        >
          {words.buildAnyway}
        </button>
      )}

      {/*
        What the source is, and then what its peak is. The peak alone was on screen and the
        rest was written and shown nowhere — and without the frame and the rate, a peak in
        megabits is a number with nothing to compare it to.
      */}
      {preview && (
        <p data-testid="source-is">
          {fill(
            words.sourceIs,
            {
              width: preview.source.width,
              height: preview.source.height,
              fps: preview.source.fps,
              bitrate: preview.source.bitrate_bps,
            },
            t,
            lang,
          )}
        </p>
      )}

      {source && (
        <p data-testid="source-facts">{words.peakIs.replace("{peak}", bitrate(source.peak_bps))}</p>
      )}

      {working && (
        <p role="status" data-testid="working">
          {words.working}
        </p>
      )}

      {preview && <Provenance preview={preview} />}

      {/*
        What the core wanted to say and had nowhere to say it. `NoticeProbeFailed`,
        `NoticeNoHardwareFound`, `NoticeProbeUncalibrated` and `NoticeMeasurementBorrowed`
        were all being produced and all being dropped on the floor here — so a ladder built
        on a probe that failed looked exactly like one built on a probe that worked.
      */}
      {preview && preview.notices.length > 0 && (
        <ul data-testid="notices">
          {preview.notices.map((notice, i) => (
            <li key={i} role="note">
              {renderDetail(notice, t, lang)}
            </li>
          ))}
        </ul>
      )}

      {offer && (
        <MeasureOffer
          preview={offer}
          running={measuring}
          onStart={() => {
            setMeasuring(true);
            ipc
              .qualityMeasureStart({ path })
              .then((id) => {
                measuringId.current = id;
              })
              .catch((e: AppError) => {
                setError(e);
                measuringId.current = null;
                setMeasuring(false);
              });
          }}
        />
      )}

      {preview && (
        <RungEditor
          rungs={rungs}
          source={preview.source}
          onChange={setRungs}
          left_out={leftOut}
          onToggle={(index) =>
            setLeftOut((was) => {
              const next = new Set(was);
              if (!next.delete(index)) next.add(index);
              return next;
            })
          }
        />
      )}

      {/*
        What the set will be called on the server. Offered rather than decided: the
        guess comes from the file's own name, and `фильм 22.mp4` guesses down to `-22`,
        which is not what anybody meant and is not obvious until the set is somewhere
        nobody expected.
      */}
      {/*
        T528 — an explicit link to an existing medium, instead of leaving it to a slug
        guessed from the file name that happens (or does not) to match one already there.
        Shown only once a server is known and its library actually has something to offer;
        with no server chosen the pattern already used below for the build button applies
        here too — a hint instead of silently omitting a control.
      */}
      {serverId && existingMedia.length > 0 && (
        <label>
          {words.attachToExisting}
          <select
            value={selectedMediaId}
            onChange={(e) => setSelectedMediaId(e.target.value)}
            aria-label={words.attachToExisting}
          >
            <option value="">{words.attachToNewSet}</option>
            {existingMedia.map((m) => (
              <option key={m.id} value={m.id}>
                {m.title} ({m.slug})
              </option>
            ))}
          </select>
        </label>
      )}
      {!serverId && (
        <p className="muted" data-testid="attach-no-server">
          {words.attachChooseServer}
        </p>
      )}

      {selectedMedia ? (
        <p data-testid="attach-slug">{fill(words.attachSlug, { slug: selectedMedia.slug }, t, lang)}</p>
      ) : (
        <label>
          {words.setName}
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label={words.setName}
          />
        </label>
      )}

      {/*
        T522 — two fields the core already reads off `LadderRequest` and this screen never
        gave anyone a way to fill in. Folded away, following `ServerForm.tsx`'s pattern for
        the rarely-needed: most files need neither, and both mostly matter for material
        that was upscaled or shot stereoscopic, which is not the common case.
      */}
      <details className="form__extra" data-testid="ladder-advanced">
        <summary>{words.advanced}</summary>
        <label>
          {words.nativeHeight}
          <input
            type="number"
            value={nativeHeightInput}
            onChange={(e) => setNativeHeightInput(e.target.value)}
            placeholder={words.nativeHeightPlaceholder}
            aria-label={words.nativeHeight}
          />
        </label>
        <label>
          {words.declaredLayout}
          <select
            value={declaredLayout}
            onChange={(e) =>
              setDeclaredLayout(e.target.value as "" | "Flat" | "SideBySide" | "OverUnder")
            }
            aria-label={words.declaredLayout}
          >
            <option value="">{words.declaredLayoutUnknown}</option>
            <option value="Flat">{words.declaredLayoutFlat}</option>
            <option value="SideBySide">{words.declaredLayoutSideBySide}</option>
            <option value="OverUnder">{words.declaredLayoutOverUnder}</option>
          </select>
        </label>
      </details>

      <button
        type="button"
        disabled={blocked !== null || building || !serverId || rungs.length === leftOut.size}
        data-testid="build"
        onClick={() => doBuild()}
      >
        {building ? words.building : words.build}
      </button>
      {/* Taking another film's measurement, and getting back out of it (T427, T428). Above
          the evidence rather than below: somebody with no measurement at all is choosing
          whether to spend half an hour, and that choice comes before the numbers do. */}
      {preview && (
        <Borrow
          path={path}
          borrowedFrom={preview.borrowed_from}
          measuredHere={preview.from === "measured"}
          sourceKey={preview.measurement_key}
          codec={preview.codec}
          onChanged={() => void loadPlan()}
        />
      )}

      {/* The evidence behind the rungs, folded away (T420, T421). Only where there is a
          measurement to look into: a ladder from the formula measured nothing. */}
      {preview?.measurement_key && (
        <MeasuredPoints sourceKey={preview.measurement_key} codec={preview.codec} />
      )}

      {measured.length > 0 && (
        <ul data-testid="measure-notices">
          {measured.map((notice, i) => (
            <li key={i} role="note">
              {renderDetail(notice, t, lang)}
            </li>
          ))}
        </ul>
      )}
      {blocked && (
        <p role="note" data-testid="build-blocked">
          {blocked.code === "NO_RUNGS" ? words.buildBlockedEmpty : words.buildBlocked}
        </p>
      )}
      {/*
        A button that does nothing teaches people the application is broken. This one used
        to return on the spot when no server was chosen — no refusal, no explanation, and
        no way to tell it apart from a set that failed to build.
      */}
      {!serverId && (
        <p role="note" data-testid="build-no-server">
          {words.noServer}
        </p>
      )}
    </div>
  );
}
