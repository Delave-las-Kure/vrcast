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
import { RungEditor } from "./RungEditor";
import { useActiveServer } from "../servers/store";
import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import { ipc, onTaskDone } from "../../shared/ipc";
import type {
  AppError,
  LadderPreview,
  MeasurePreview,
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
  const words = t.ui.ladder;
  const left = Math.max(0, preview.points - preview.already_measured);
  const minutes = Math.max(1, Math.round(preview.about_seconds / 60));

  return (
    <section aria-label={words.measureTitle}>
      <h3>{words.measureTitle}</h3>
      <p>{words.measureExplain}</p>
      <p data-testid="how-long">
        {preview.already_measured > 0
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
      */}
      <p data-testid="estimate-from">
        {preview.estimate_from_points > 0
          ? words.estimateFromThisMachine.replace("{points}", String(preview.estimate_from_points))
          : words.estimateFromModel}
      </p>
      <button type="button" onClick={onStart} disabled={running}>
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
/**
 * The medium's directory, guessed from the file's own name.
 *
 * A guess, and only until the person is given somewhere to say otherwise: it is the same
 * name the upload screen offers, so a set built here lands beside the file it came from.
 */
function slugOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  return name
    .replace(/\.[^.]+$/, "")
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .toLowerCase();
}

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
  const [measuring, setMeasuring] = useState(false);
  // Tied to the file rather than to what is running: this must go false when the screen is
  // put away or asked about another file, and at no other moment.
  const alive = useRef(true);
  const [name, setName] = useState(slug ?? "");
  // Which rungs the person has left out. By the rung's own index rather than by position,
  // so that editing a bitrate — which rebuilds the array — does not silently move the
  // choice onto a different rung.
  const [leftOut, setLeftOut] = useState<ReadonlySet<number>>(new Set());
  const [building, setBuilding] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, [path]);

  /**
   * Ask the core for the ladder and put its answer on screen.
   *
   * One function for both the first load and the reload after a measurement. It used to be
   * written out twice, in two effects, and the two could drift — one of them already had a
   * line the other did not.
   */
  const loadPlan = useCallback(async (): Promise<LadderPreview | null> => {
    setWorking(true);
    try {
      const answer = await ipc.ladderPlan({ path });
      if (!alive.current) return null;
      setPreview(answer);
      setRungs(answer.plan.rungs);
      if (answer.from !== "formula") setOffer(null);
      return answer;
    } catch (e) {
      if (alive.current) setError(e as AppError);
      return null;
    } finally {
      if (alive.current) setWorking(false);
    }
  }, [path]);

  useEffect(() => {
    if (!path) return;
    setError(null);

    void loadPlan().then((answer) => {
      // The offer to measure is only worth fetching when there is nothing measured yet:
      // it runs the complexity probe, and running that on a screen that already has an
      // answer would cost seconds for nothing.
      if (!alive.current || answer?.from !== "formula") return;
      ipc
        .qualityMeasurePreview({ path })
        .then((o) => {
          if (alive.current) setOffer(o);
        })
        .catch(() => undefined);
    });

    // The source's own numbers, separately: measuring the peaks reads every packet and is
    // slower than planning, and there is no reason to make the rungs wait for it.
    ipc
      .ladderMeasure(path)
      .then((m) => {
        if (alive.current) setSource(m);
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
      void loadPlan().finally(() => {
        if (alive.current) setMeasuring(false);
      });
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [path, loadPlan]);

  const blocked = preview?.verdict.not_buildable ?? null;

  return (
    <div>
      <h2>{words.rungs}</h2>
      <p>{words.explain}</p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

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
      <label>
        {words.setName}
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label={words.setName}
        />
      </label>

      <button
        type="button"
        disabled={blocked !== null || building || !serverId || rungs.length === leftOut.size}
        data-testid="build"
        onClick={() => {
          if (!preview || !serverId) return;
          setBuilding(true);
          ipc
            .ladderBuild({
              server_id: serverId,
              path,
              slug: name.trim() || slugOf(path),
              // Only what was asked for. The core names each variant by its own megabits
              // (`film_22.mp4`), not by its place in the list, so a gap in the numbering
              // costs nothing.
              rungs: rungs.filter((rung) => !leftOut.has(rung.index)),
            })
            .catch((e: AppError) => setError(e))
            .finally(() => setBuilding(false));
        }}
      >
        {building ? words.building : words.build}
      </button>
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
