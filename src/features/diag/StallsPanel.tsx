/**
 * Почему у зрителя встаёт картинка — с числами, на которых держится вывод (FR-072).
 *
 * **Вывод без чисел нечем оспорить, а он бывает неверен.** Поэтому рядом с «не хватает канала
 * у зрителя» стоит и полученное против реального времени, и сама скорость, и сколько
 * отрезков плеер перепрыгнул. Человек, который видит эти числа, может не согласиться —
 * человек, который видит одну фразу, может только поверить.
 *
 * **Две скорости показаны обе, и подписаны.** Внутри закачек всегда выходит больше, чем по
 * стенным часам: это та же вода, налитая за меньшее время. Канал зрителя — второе число, и
 * спутать их значит посоветовать человеку с исправной линией менять провайдера.
 *
 * **Не-зрители показаны, а не выброшены.** Кеш, набирающий себе пару отрезков, и наши
 * собственные проверки — это то, что человек мог бы принять за зрителя; видеть, что их
 * узнали, полезнее, чем не видеть их вовсе.
 */

import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import { formatBitrate } from "../../shared/i18n/format";
import type { Stalls, Watcher } from "../../shared/contract";

/** Число с двумя знаками, либо прочерк. Прочерк — не ноль: ноль читается как измерение. */
function ratio(value: number | null, nothing: string): string {
  return value === null ? nothing : `${value.toFixed(2)}×`;
}

function speed(value: number | null, lang: "ru" | "en", nothing: string): string {
  return value === null ? nothing : formatBitrate(value * 1_000_000, lang);
}

export function StallsPanel({ stalls }: { stalls: Stalls }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.diag;
  const nothing = words.notDetermined;

  return (
    <section className="diag-stalls">
      <h3>{words.stallsTitle}</h3>

      <p className="diag-load" data-testid="stalls-load">
        {words.stallsLoad}: {words.stallsLoadCpu} {Math.round(stalls.load.cpu_busy * 100)}% ·{" "}
        {words.stallsLoadDisk} {stalls.load.disk_read_mb_s.toFixed(1)} MB/s · {words.stallsLoadOut}{" "}
        {formatBitrate(stalls.load.out_mbit_s * 1_000_000, lang)}
        {stalls.load.capacity_mbit_s > 0 ? (
          <>
            {" "}
            {words.stallsLoadCapacity}{" "}
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
                      {words.stallsWatching}: {w.watching}
                    </>
                  )}
                </p>

                {verdict && (
                  <p className="diag-verdict" data-testid={`verdict-${w.client_ip}`}>
                    {renderDetail(verdict.say, t, lang)}
                  </p>
                )}

                {/* Те же числа отдельным списком, а не только внутри фразы: по ним
                    сравнивают зрителей между собой, и глазами это делается по столбцу. */}
                <dl className="diag-figures">
                  <dt>{words.stallsRatio}</dt>
                  <dd data-testid={`ratio-${w.client_ip}`}>{ratio(w.content_ratio, nothing)}</dd>
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

      {stalls.watchers.some((w) => w.in_download_mbit_s !== null) && (
        <p className="diag-hint">{words.stallsInDownloadHint}</p>
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
