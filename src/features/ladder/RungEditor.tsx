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
 *
 * **And a rung can be left out.** The application works the ladder out and offers it; which
 * of the rungs are actually made is the person's to say (owner, 2026-08-28). Every rung is
 * hours of encoding and a copy on the server, so "all of them or edit the numbers by hand"
 * was not a real choice. What is checked — and what is built — is what is left in: leaving
 * out a middle rung widens the gap over its neighbour, and the core objects to that while
 * the person is still looking at it, not after they have agreed to the work.
 */

import { useEffect, useMemo, useState } from "react";

import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type {
  LadderVerdict,
  Objection,
  Quality,
  Rung,
  RungReason,
  SourceFacts,
} from "../../shared/contract";

/** Megabits, as everything in this project is counted. */
function mbps(bps: number): string {
  return `${Math.round(bps / 1_000_000)} Mbit/s`;
}

/**
 * What is known about this rung, and how it came to be known.
 *
 * **Three states, not two** (T419, FR-145). "Measured here" and "borrowed from another
 * file" used to collapse into one sentence and one `data-measured="yes"`, so a set that was
 * partly measured and partly lent looked uniform, and the only mark of the borrowing was a
 * line at the top of the whole set. But the borrowing is not a property of the set: a rung
 * measured here sitting next to one lent from a neighbouring episode is exactly the case
 * where the difference matters, and the heading cannot say it.
 */
function qualityText(
  quality: Quality,
  words: { notMeasured: string; vmafIs: string; vmafBorrowed: string },
): { text: string; measured: boolean; borrowed: boolean } {
  if (quality.state === "not_measured") {
    return { text: words.notMeasured, measured: false, borrowed: false };
  }
  const borrowed = quality.state === "borrowed";
  const wording = borrowed ? words.vmafBorrowed : words.vmafIs;
  return {
    text: wording.replace("{value}", (quality.vmaf_x100 / 100).toFixed(2)),
    measured: true,
    borrowed,
  };
}

/**
 * Why a rung looks the way it does, in words and with the numbers it is about (T418).
 *
 * **The numbers come from the rung, not from the reason.** The core sends a bare list of
 * codes; what a code is *about* — how far down, to what height, how much was cut — is in the
 * rung itself and, for a step, in the one above it. Assembling the sentence here rather than
 * in the core is the same rule as everywhere else: the core emits codes, the wordings live
 * in the catalogues (FR-105).
 */
function reasonText(
  reason: RungReason,
  rung: Rung,
  above: Rung | undefined,
  words: Record<RungReason, string>,
): string {
  const times = above ? above.bitrate_bps / Math.max(1, rung.bitrate_bps) : 1;
  return words[reason]
    .replace("{mbps}", String(Math.round(rung.bitrate_bps / 1_000_000)))
    .replace("{height}", String(rung.height))
    .replace("{width}", String(rung.width))
    .replace("{times}", times.toFixed(1));
}

/** The wordings an objection needs, named rather than taken as any string at all: the
 *  ladder's catalogue now holds a nested block too, and `Record<string, string>` would be a
 *  lie about it. */
interface ObjectionWords {
  objectionAboveSource: string;
  objectionBufsize: string;
  objectionLevel: string;
  objectionOrder: string;
  objectionStep: string;
  stepTooBig: string;
  stepTooSmall: string;
}

/** An objection in words. The core sends a code and numbers; the sentence is made here. */
function objectionText(objection: Objection, words: ObjectionWords): string {
  if ("RungAboveSource" in objection) {
    return words.objectionAboveSource.replace(
      "{index}",
      String(objection.RungAboveSource.index + 1),
    );
  }
  if ("BufsizeTooLarge" in objection) {
    return words.objectionBufsize.replace("{index}", String(objection.BufsizeTooLarge.index + 1));
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
  left_out,
  onToggle,
}: {
  rungs: Rung[];
  source: SourceFacts;
  onChange?: (rungs: Rung[]) => void;
  /** The rungs, by their own index, that are not to be built. */
  left_out?: ReadonlySet<number>;
  onToggle?: (index: number) => void;
}) {
  const t = useT();
  const words = t.ui.ladder;
  const [verdict, setVerdict] = useState<LadderVerdict | null>(null);

  // What will actually be built. The check has to be about these and not about the whole
  // list: leaving a rung out is what widens a gap, and a check that looked at the rungs
  // nobody asked for would stay quiet about it.
  const building = useMemo(
    () => rungs.filter((rung) => !left_out?.has(rung.index)),
    [rungs, left_out],
  );

  // Checked on every change of the rungs, including the first time they arrive. The core's
  // check touches neither a file nor a server, so this cannot fall behind the typing.
  useEffect(() => {
    let alive = true;
    ipc
      .ladderValidate(building, source)
      .then((answer) => {
        if (alive) setVerdict(answer);
      })
      .catch(() => {
        if (alive) setVerdict(null);
      });
    return () => {
      alive = false;
    };
  }, [building, source]);

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
            {onToggle && <th>{words.columnBuild}</th>}
            <th>{words.columnBitrate}</th>
            <th>{words.columnSize}</th>
            <th>{words.columnQuality}</th>
            <th>{words.columnWhy}</th>
          </tr>
        </thead>
        <tbody>
          {rungs.map((rung, index) => {
            const quality = qualityText(rung.quality, words);
            const out = left_out?.has(rung.index) ?? false;
            return (
              <tr
                key={rung.index}
                data-testid={`rung-${rung.index}`}
                data-left-out={out ? "yes" : "no"}
              >
                {onToggle && (
                  <td>
                    <input
                      type="checkbox"
                      checked={!out}
                      aria-label={words.buildThisRung.replace(
                        "{mbps}",
                        String(Math.round(rung.bitrate_bps / 1_000_000)),
                      )}
                      onChange={() => onToggle(rung.index)}
                    />
                  </td>
                )}
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
                <td
                  data-measured={quality.measured ? "yes" : "no"}
                  data-borrowed={quality.borrowed ? "yes" : "no"}
                >
                  {quality.text}
                </td>
                {/* Every reason the core gave, not the first of them: a rung is usually
                    the result of more than one decision, and showing one of them would be
                    picking which half of the answer to withhold. */}
                <td data-testid={`why-${rung.index}`}>
                  <ul>
                    {rung.reasons.map((reason) => (
                      <li key={reason}>
                        {reasonText(reason, rung, rungs[index - 1], words.reasons)}
                      </li>
                    ))}
                  </ul>
                </td>
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

export { objectionText, qualityText, reasonText };
