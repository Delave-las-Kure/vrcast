/**
 * T318 — where a file's peaks actually are, rather than one number (FR-073).
 *
 * **"A peak of 150 Mbit/s" with no place attached is grounds for alarm and for nothing else.**
 * Told where, a person can open the film at that second, see the explosion in it, and know that
 * re-encoding is needed rather than guess. So every window carries its timecode.
 *
 * **What is counted is the peak of a ten-second window, and that is what is compared with a
 * viewer's connection.** A one-second spike is swallowed by any buffer; an average over the
 * film hides everything. Ten seconds is about what a player's buffer holds, and a stretch that
 * long which the connection cannot carry drains the buffer dry.
 *
 * The server is not touched here at all: the question is about the file, and the answer is the
 * same before the upload as after. Before is more use.
 */

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { formatBitrate, formatDuration } from "../../shared/i18n/format";
import { ipc } from "../../shared/ipc";
import type { AppError, BitrateWindow, Peaks } from "../../shared/contract";

/** How many times above the average a peak has to be. Below this a file counts as even. */
const PEAK_WORTH_MENTIONING = 1.5;

function Where({ window: w }: { window: BitrateWindow | null }) {
  const t = useT();
  const { lang } = useLang();
  if (!w) return <>{t.ui.diag.notDetermined}</>;
  return (
    <>
      {formatBitrate(w.bitrate_bps, lang)} {t.ui.diag.bitrateAt} {formatDuration(w.at_s)}
    </>
  );
}

export function BitratePeaks({ path }: { path?: string }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.diag;

  const [chosen, setChosen] = useState<string | null>(path ?? null);
  const [peaks, setPeaks] = useState<Peaks | null>(null);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  async function pick() {
    const picked = await open({ multiple: false, directory: false });
    if (typeof picked !== "string") return;
    setChosen(picked);
    setPeaks(null);
    setError(null);
    setAsking(true);
    try {
      setPeaks(await ipc.diagBitrate(picked));
    } catch (e) {
      setError(e as AppError);
    } finally {
      setAsking(false);
    }
  }

  const overAverage =
    peaks && peaks.wide && peaks.average_bps > 0
      ? peaks.wide.bitrate_bps / peaks.average_bps
      : null;

  return (
    <section className="diag-bitrate">
      <h3>{words.bitrateTitle}</h3>
      <p className="diag-hint">{words.bitrateHint}</p>

      <button type="button" onClick={pick}>
        {words.bitratePick}
      </button>
      {chosen && <p className="diag-chosen">{chosen}</p>}

      {asking && <p>{words.asking}</p>}
      {error && <ErrorNotice error={error} />}

      {peaks && (
        <>
          <dl className="diag-figures">
            <dt>{words.bitrateAverage}</dt>
            <dd data-testid="bitrate-average">{formatBitrate(peaks.average_bps, lang)}</dd>
            <dt>{words.bitrateMedian}</dt>
            <dd>{formatBitrate(peaks.median_bps, lang)}</dd>
            <dt>{words.bitratePeak1}</dt>
            <dd data-testid="bitrate-peak-1">
              <Where window={peaks.one_second} />
            </dd>
            <dt>{words.bitratePeak10}</dt>
            <dd data-testid="bitrate-peak-10">
              <Where window={peaks.wide} />
            </dd>
          </dl>

          {overAverage !== null &&
            (overAverage >= PEAK_WORTH_MENTIONING ? (
              <div data-testid="bitrate-peaky">
                <p>{words.bitratePeakOverAverage(Number(overAverage.toFixed(1)))}</p>
                <p>{words.bitrateAdvice}</p>
              </div>
            ) : (
              <p data-testid="bitrate-even">{words.bitrateEven}</p>
            ))}

          {peaks.worst_wide.length > 0 && (
            <>
              <h4>{words.bitrateWorst}</h4>
              <ul className="diag-worst-windows">
                {peaks.worst_wide.map((w) => (
                  <li key={w.at_s} data-testid={`window-${w.at_s}`}>
                    {formatDuration(w.at_s)} — {formatBitrate(w.bitrate_bps, lang)}
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      )}
    </section>
  );
}
