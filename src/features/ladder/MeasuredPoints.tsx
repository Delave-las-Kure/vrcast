/**
 * T420, T421 — what was actually measured, and what was thrown away above the target.
 *
 * **The one answer to "why does the top stop at 22 and not 35".** The core encodes a grid of
 * bitrate × height, scores each with VMAF, takes the upper hull and cuts it where the quality
 * stops improving. Every number of that is worked out and stored — and none of it reached a
 * screen, so the ladder arrived as an assertion. A person who thinks the top is too low had
 * nothing to argue with except the number they disagreed with.
 *
 * **Folded away, not spread out.** The screen already carries the source, the provenance, the
 * offer to measure, the rungs, the set's name and the build button. This is the evidence
 * behind one of those, wanted by the person who doubts it and by nobody else — so it opens
 * when asked and takes one line when not.
 */

import { useEffect, useState } from "react";

import type { MeasurementView } from "../../shared/contract";
import { ipc } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

/** Whether this measured point ended up as a rung. */
function chosen(view: MeasurementView, mbps: number, height: number): boolean {
  return (view.selection?.rungs ?? []).some(
    (rung) => rung.bitrate_mbps === mbps && rung.height === height,
  );
}

export function MeasuredPoints({ sourceKey, codec }: { sourceKey: string; codec: string }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.ladder;
  const [view, setView] = useState<MeasurementView | null>(null);

  useEffect(() => {
    let alive = true;
    setView(null);
    ipc
      .qualityMeasureResult(sourceKey, codec)
      // Silent on failure: this is the evidence behind the rungs, not the rungs. Spoiling
      // the screen because an aside would not load helps nobody.
      .then((answer) => {
        if (alive) setView(answer);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [sourceKey, codec]);

  if (!view || view.points.length === 0) return null;

  const above = view.selection?.above_target ?? [];

  return (
    <details data-testid="measured-points">
      <summary>{fill(words.measuredTitle, { points: view.points.length }, t, lang)}</summary>

      <table>
        <thead>
          <tr>
            <th>{words.columnBitrate}</th>
            <th>{words.columnSize}</th>
            <th>{words.measuredColumnVmaf}</th>
            <th>{words.measuredColumnActual}</th>
          </tr>
        </thead>
        <tbody>
          {view.points.map((point) => {
            const taken = chosen(view, point.bitrate_mbps, point.height);
            return (
              <tr
                key={`${point.bitrate_mbps}x${point.height}`}
                data-chosen={taken ? "yes" : "no"}
                title={taken ? words.measuredChosen : undefined}
              >
                <td>{point.bitrate_mbps}</td>
                <td>{point.height}</td>
                <td>{point.vmaf.toFixed(2)}</td>
                {/* What the encoder actually produced, beside what it was asked for. The two
                    differ, and where they differ a lot is where a rung costs more than its
                    number says. */}
                <td>{(point.actual_bps / 1_000_000).toFixed(1)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {/* T421. Without this a person looks for the bitrate that went missing: the grid was
          measured up to 35 and the ladder tops out at 22, and nothing said the rest was
          dropped on purpose. */}
      {above.length > 0 && (
        <p role="note" data-testid="dropped-above">
          {fill(
            words.droppedAbove,
            { list: above.map((point) => `${point.bitrate_mbps} Mbit/s`).join(", ") },
            t,
            lang,
          )}
        </p>
      )}
    </details>
  );
}
