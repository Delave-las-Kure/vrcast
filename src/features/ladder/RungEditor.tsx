/**
 * T200, T241 — the rungs, with what each is worth and what is wrong with it.
 *
 * **Objections appear while a person edits, not after they press build** (FR-044). Checking
 * is a pure function in the core: it costs nothing, waits on nothing, and there is no
 * reason to save it up. Learning that a rung is impossible after agreeing to hours of
 * encoding is learning it too late.
 *
 * **A rung says whether anybody has measured it** (FR-145). A number from the formula shown
 * the way a measured one is shown would be worse than no number at all: it is a guess
 * wearing the clothes of a fact.
 */

import { useEffect, useState } from "react";

import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type {
  LadderVerdict,
  Objection,
  Quality,
  Rung,
  SourceFacts,
} from "../../shared/contract";

/** Megabits, as everything in this project is counted. */
function mbps(bps: number): string {
  return `${Math.round(bps / 1_000_000)} Mbit/s`;
}

function qualityText(
  quality: Quality,
  words: { notMeasured: string; vmafIs: string },
): { text: string; measured: boolean } {
  if (quality.state === "not_measured") {
    return { text: words.notMeasured, measured: false };
  }
  return {
    text: words.vmafIs.replace("{value}", (quality.vmaf_x100 / 100).toFixed(2)),
    measured: true,
  };
}

/** An objection in words. The core sends a code and numbers; the sentence is made here. */
function objectionText(objection: Objection, words: Record<string, string>): string {
  if ("RungAboveSource" in objection) {
    return words.objectionAboveSource.replace(
      "{index}",
      String(objection.RungAboveSource.index + 1),
    );
  }
  if ("BufsizeTooLarge" in objection) {
    return words.objectionBufsize.replace(
      "{index}",
      String(objection.BufsizeTooLarge.index + 1),
    );
  }
  if ("LevelExceeded" in objection) {
    return words.objectionLevel
      .replace("{index}", String(objection.LevelExceeded.index + 1))
      .replace("{level}", objection.LevelExceeded.level);
  }
  if ("OutOfOrder" in objection) {
    return words.objectionOrder.replace("{index}", String(objection.OutOfOrder.index + 1));
  }
  const step = objection.BadStep;
  return words.objectionStep
    .replace("{index}", String(step.index + 1))
    .replace("{times}", step.times.toFixed(1))
    .replace("{tooMuch}", step.times > 2 ? words.stepTooBig : words.stepTooSmall);
}

export function RungEditor({
  rungs,
  source,
  onChange,
}: {
  rungs: Rung[];
  source: SourceFacts;
  onChange?: (rungs: Rung[]) => void;
}) {
  const t = useT();
  const words = t.ui.ladder;
  const [verdict, setVerdict] = useState<LadderVerdict | null>(null);

  // Checked on every change of the rungs, including the first time they arrive. The core's
  // check touches neither a file nor a server, so this cannot fall behind the typing.
  useEffect(() => {
    let alive = true;
    ipc
      .ladderValidate(rungs, source)
      .then((answer) => {
        if (alive) setVerdict(answer);
      })
      .catch(() => {
        if (alive) setVerdict(null);
      });
    return () => {
      alive = false;
    };
  }, [rungs, source]);

  function edit(index: number, bitrateMbps: number) {
    if (!onChange) return;
    const next = rungs.map((rung, i) =>
      i === index
        ? {
            ...rung,
            bitrate_bps: Math.max(1, Math.round(bitrateMbps)) * 1_000_000,
            // **A rung moved by hand leaves the measured grid.** Nobody has looked at what
            // it is worth at its new value, and saying otherwise would be the one lie this
            // screen could tell that a person would believe.
            quality: { state: "not_measured" } as Quality,
          }
        : rung,
    );
    onChange(next);
  }

  return (
    <section aria-label={words.rungs}>
      <h3>{words.rungs}</h3>
      <table>
        <thead>
          <tr>
            <th>{words.columnBitrate}</th>
            <th>{words.columnSize}</th>
            <th>{words.columnQuality}</th>
          </tr>
        </thead>
        <tbody>
          {rungs.map((rung, index) => {
            const quality = qualityText(rung.quality, words);
            return (
              <tr key={rung.index} data-testid={`rung-${rung.index}`}>
                <td>
                  {onChange ? (
                    <input
                      type="number"
                      min={1}
                      aria-label={`${words.columnBitrate} ${index + 1}`}
                      value={Math.round(rung.bitrate_bps / 1_000_000)}
                      onChange={(e) => edit(index, Number(e.target.value))}
                    />
                  ) : (
                    mbps(rung.bitrate_bps)
                  )}
                </td>
                <td>
                  {rung.width}×{rung.height}
                </td>
                <td data-measured={quality.measured ? "yes" : "no"}>{quality.text}</td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {verdict && verdict.objections.length > 0 && (
        <div role="alert" aria-label={words.objections}>
          <h4>{words.objections}</h4>
          <ul>
            {verdict.objections.map((objection, i) => (
              <li key={i}>{objectionText(objection, words)}</li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

export { objectionText, qualityText };
