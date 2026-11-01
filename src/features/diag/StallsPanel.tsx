/**
 * Why a viewer's picture stops — with the numbers the reading rests on (FR-072).
 *
 * **A verdict with no numbers cannot be argued with, and it is sometimes wrong.** So beside
 * "the viewer's connection is not enough" stand what was delivered against real time, the
 * speed itself, and how many segments the player skipped. Somebody who sees those numbers can
 * disagree; somebody who sees one sentence can only believe it.
 *
 * **Both speeds are shown, and both are labelled.** Inside the downloads it always comes out
 * higher than by the wall clock: the same water poured in less time. The viewer's connection
 * is the second number, and confusing the two means telling somebody with a perfectly good
 * line to change provider.
 *
 * **Non-viewers are shown, not thrown away.** A cache taking a couple of segments for itself,
 * and our own checks, are exactly what a person might mistake for a viewer; seeing that they
 * were recognised is more use than not seeing them at all.
 */

import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import { formatBitrate, formatNumber } from "../../shared/i18n/format";
import type { Stalls, Watcher } from "../../shared/contract";

/** A number to two places, or a dash. A dash is not a zero: a zero reads as a measurement. */
function ratio(value: number | null, lang: "ru" | "en", nothing: string): string {
  return value === null ? nothing : `${formatNumber(value, lang)}×`;
}

function speed(value: number | null, lang: "ru" | "en", nothing: string): string {
  return value === null ? nothing : formatBitrate(value * 1_000_000, lang);
}

/** The core's own thresholds for the live reading (`domain::stalls`: `LIVE_SENDING`,
 *  `LIVE_HELD`) — below the first the connection hardly sent at all; above the second the
 *  viewer's side was full. */
const LIVE_SENDING = 0.1;
const LIVE_HELD = 0.5;

/**
 * The live reading, or — when nothing came through — why, in a few words (tour 0.3.0, G03).
 *
 * A bare «—» beside «Сейчас по соединению» said neither "nothing" nor "not measured". Nothing
 * arriving has three plain reasons the reading itself tells apart: nothing was asked for, the
 * player was full and not taking it, or it was being sent and did not get through.
 */
function liveLine(
  live: NonNullable<Watcher["live"]>,
  lang: "ru" | "en",
  words: { stallsLiveIdle: string; stallsLiveHeld: string; stallsLiveStuck: string },
): string {
  const value = live.busy_mbit_s ?? live.mbit_s;
  if (value > 0) return formatBitrate(value * 1_000_000, lang);
  if ((live.busy_share ?? 0) < LIVE_SENDING) return words.stallsLiveIdle;
  if ((live.held_share ?? 0) > LIVE_HELD) return words.stallsLiveHeld;
  return words.stallsLiveStuck;
}

export function StallsPanel({
  stalls,
  titles = {},
}: {
  stalls: Stalls;
  /** Films by their directory name, so a viewer reads the film's title, not its slug (T705). */
  titles?: Record<string, string>;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.diag;
  const nothing = words.notDetermined;

  return (
    <section className="diag-stalls">
      <h3>{words.stallsTitle}</h3>

      <p className="diag-load" data-testid="stalls-load">
        {words.stallsLoad}: {words.stallsLoadCpu} {Math.round(stalls.load.cpu_busy * 100)}% ·{" "}
        {words.stallsLoadDisk} {formatNumber(stalls.load.disk_read_mb_s, lang, 1)} {words.unitMBps}{" "}
        · {words.stallsLoadOut} {formatBitrate(stalls.load.out_mbit_s * 1_000_000, lang)}
        {stalls.load.capacity_mbit_s > 0 ? (
          <>
            {" "}
            {stalls.load.capacity_by === "tariff"
              ? words.stallsLoadTariff
              : words.stallsLoadCapacity}{" "}
            {formatBitrate(stalls.load.capacity_mbit_s * 1_000_000, lang)}
          </>
        ) : (
          <span className="diag-hint"> ({words.stallsCapacityUnknown})</span>
        )}
      </p>

      {stalls.watchers.length === 0 ? (
        <p data-testid="stalls-none">{words.stallsNoViewers}</p>
      ) : (
        <ul className="diag-watchers">
          {stalls.watchers.map((w: Watcher, i: number) => {
            const verdict = stalls.verdicts[i];
            return (
              <li
                key={w.client_ip}
                data-testid={`watcher-${w.client_ip}`}
                data-cause={verdict?.cause}
              >
                <p className="diag-watcher-who">
                  {w.client_ip}
                  {w.watching && (
                    <>
                      {" · "}
                      {words.stallsWatching}: {titles[w.watching] ?? w.watching}
                    </>
                  )}
                </p>

                {verdict && (
                  <p className="diag-verdict" data-testid={`verdict-${w.client_ip}`}>
                    {renderDetail(verdict.say, t, lang)}
                  </p>
                )}

                {/* The same numbers as a list of their own, not only inside the sentence:
                    they are what viewers get compared by, and by eye that is done down a
                    column. */}
                <dl className="diag-figures">
                  <dt>{words.stallsRatio}</dt>
                  <dd data-testid={`ratio-${w.client_ip}`}>
                    {ratio(w.content_ratio, lang, nothing)}
                  </dd>
                  <dt>{words.stallsLink}</dt>
                  <dd data-testid={`link-${w.client_ip}`}>
                    {speed(w.mbit_s, lang, nothing)}
                    {w.in_download_mbit_s !== null && (
                      <>
                        {" ("}
                        {words.stallsInDownload} {speed(w.in_download_mbit_s, lang, nothing)}
                        {")"}
                      </>
                    )}
                  </dd>
                  <dt>{words.stallsNeeds}</dt>
                  <dd data-testid={`needs-${w.client_ip}`}>
                    {speed(w.need_mbit ?? null, lang, nothing)}
                  </dd>
                  {w.live && (
                    <>
                      <dt>{words.stallsLive}</dt>
                      <dd data-testid={`live-${w.client_ip}`}>{liveLine(w.live, lang, words)}</dd>
                    </>
                  )}
                  <dt>{words.stallsSkipped}</dt>
                  <dd>{w.skipped.length}</dd>
                  <dt>{words.stallsRestarts}</dt>
                  <dd>{w.restarts}</dd>
                </dl>
              </li>
            );
          })}
        </ul>
      )}

      {stalls.set_aside.length > 0 && (
        <>
          <h4>{words.stallsSetAside}</h4>
          <ul className="diag-set-aside">
            {stalls.set_aside.map((a) => (
              <li key={a.client_ip} data-testid={`aside-${a.client_ip}`}>
                {a.client_ip} —{" "}
                {a.why === "our_own_check"
                  ? words.stallsOurOwnCheck
                  : words.stallsTooLittle(a.why.too_little.segments)}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
