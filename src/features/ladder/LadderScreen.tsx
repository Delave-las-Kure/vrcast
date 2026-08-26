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

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { ErrorNotice } from "../shared/ErrorNotice";
import { RungEditor } from "./RungEditor";
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
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
          ? words.estimateFromThisMachine.replace(
              "{points}",
              String(preview.estimate_from_points),
            )
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
export function LadderPage() {
  const t = useT();
  const words = t.ui.ladder;
  const [path, setPath] = useState<string | null>(null);

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
    if (typeof chosen === "string") setPath(chosen);
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
  return <LadderScreen path={path} />;
}

export function LadderScreen({ path }: { path: string }) {
  const t = useT();
  const words = t.ui.ladder;

  const [source, setSource] = useState<SourceMeasured | null>(null);
  const [preview, setPreview] = useState<LadderPreview | null>(null);
  const [offer, setOffer] = useState<MeasurePreview | null>(null);
  const [rungs, setRungs] = useState<Rung[]>([]);
  const [measuring, setMeasuring] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    if (!path) return;
    let alive = true;
    setError(null);

    ipc
      .ladderPlan({ path })
      .then((answer) => {
        if (!alive) return;
        setPreview(answer);
        setRungs(answer.plan.rungs);
        // The offer to measure is only worth fetching when there is nothing measured yet:
        // it runs the complexity probe, and running that on a screen that already has an
        // answer would cost seconds for nothing.
        if (answer.from === "formula") {
          ipc
            .qualityMeasurePreview({ path })
            .then((o) => {
              if (alive) setOffer(o);
            })
            .catch(() => undefined);
        }
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });

    // The source's own numbers, separately: measuring the peaks reads every packet and is
    // slower than planning, and there is no reason to make the rungs wait for it.
    ipc
      .ladderMeasure(path)
      .then((m) => {
        if (alive) setSource(m);
      })
      .catch(() => undefined);

    return () => {
      alive = false;
    };
  }, [path]);

  const blocked = preview?.verdict.not_buildable ?? null;

  return (
    <div>
      <h2>{words.rungs}</h2>
      <p>{words.explain}</p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {source && (
        <p data-testid="source-facts">
          {words.peakIs.replace("{peak}", bitrate(source.peak_bps))}
        </p>
      )}

      {preview && <Provenance preview={preview} />}

      {offer && (
        <MeasureOffer
          preview={offer}
          running={measuring}
          onStart={() => {
            setMeasuring(true);
            ipc
              .qualityMeasureStart({ path })
              .catch((e: AppError) => {
                setError(e);
                setMeasuring(false);
              });
          }}
        />
      )}

      {preview && (
        <RungEditor rungs={rungs} source={preview.source} onChange={setRungs} />
      )}

      <button type="button" disabled={blocked !== null} data-testid="build">
        {words.build}
      </button>
      {blocked && (
        <p role="note" data-testid="build-blocked">
          {blocked.code === "NO_RUNGS" ? words.buildBlockedEmpty : words.buildBlocked}
        </p>
      )}
    </div>
  );
}
