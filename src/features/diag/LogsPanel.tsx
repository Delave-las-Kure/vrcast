/**
 * Что раздача записала за промежуток (FR-071).
 *
 * **Долгий запрос сам по себе — норма**, и здесь это сказано словами, а не спрятано в
 * пороге. Это длинная подкачка диапазона; на здоровом сервере таких большинство. Отмечены
 * только те, что при этом почти ничего не доставили, — иначе экран краснеет на исправной
 * машине, и после второго раза на него перестают смотреть.
 *
 * **206 должны преобладать.** Если чаще отдаются файлы целиком, значит куски не отдаются —
 * смотреть можно, перематывать нет, и жалоба придёт как «сломалось», не имея к сети никакого
 * отношения.
 *
 * **Упор в потолок строк говорится вслух.** Сводка, тихо покрывшая четверть запрошенного,
 * отвечает не на тот вопрос, который ей задали.
 */

import { useLang, useT } from "../../shared/i18n";
import { formatBitrate, formatBytes } from "../../shared/i18n/format";
import type { Logs } from "../../shared/contract";

export function LogsPanel({ logs }: { logs: Logs }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.diag;
  const d = logs.digest;

  if (d.requests === 0) {
    return (
      <section className="diag-logs">
        <h3>{words.logsTitle}</h3>
        <p data-testid="logs-nothing">{words.logsNothing}</p>
      </section>
    );
  }

  const ranged = d.by_status["206"] ?? 0;
  const whole = d.by_status["200"] ?? 0;
  const rangesDominate = ranged + whole === 0 ? null : ranged > whole;

  return (
    <section className="diag-logs">
      <h3>{words.logsTitle}</h3>

      {logs.reached_the_cap && (
        <p className="diag-warning" data-testid="logs-capped">
          {words.logsCapped}
        </p>
      )}

      <p>
        {words.logsRequests(d.requests)} · {words.logsAddresses(d.addresses)}
        {d.unreadable > 0 && <> · {words.logsUnreadable(d.unreadable)}</>}
        {" · "}
        {formatBytes(d.bytes_out, lang)}
      </p>

      <h4>{words.logsCodes}</h4>
      <ul className="diag-codes">
        {Object.entries(d.by_status).map(([status, times]) => (
          <li key={status} data-testid={`status-${status}`}>
            {status}: {times}
          </li>
        ))}
      </ul>
      {rangesDominate !== null && (
        <p data-testid="logs-ranges">{rangesDominate ? words.logsRangesOk : words.logsRangesBad}</p>
      )}

      <h4>{words.logsTopPaths}</h4>
      <ul>
        {d.top_paths.map((p) => (
          <li key={p.what}>
            {p.what} — {p.times}
          </li>
        ))}
      </ul>

      <h4>{words.logsTopAddresses}</h4>
      <ul>
        {d.top_addresses.map((a) => (
          <li key={a.what}>
            {a.what} — {a.times}
          </li>
        ))}
      </ul>

      <h4>{words.logsFailures}</h4>
      {d.failures.length === 0 ? (
        <p data-testid="logs-no-failures">{words.logsNoFailures}</p>
      ) : (
        <ul>
          {d.failures.map((f) => (
            <li key={`${f.status} ${f.path}`} data-testid={`failure-${f.status}`}>
              {f.status} · {f.path} — {f.times}
            </li>
          ))}
        </ul>
      )}

      {d.long_requests.length > 0 && (
        <>
          <h4>{words.logsLong}</h4>
          <p className="diag-hint" data-testid="logs-long-normal">
            {words.logsLongNormal}
          </p>
          <ul>
            {d.long_requests.map((r) => (
              <li
                key={`${r.client_ip} ${r.path} ${r.seconds}`}
                className={r.slow ? "diag-rating-watch" : undefined}
                data-testid={r.slow ? "long-slow" : "long-normal"}
              >
                {Math.round(r.seconds)} s · {formatBitrate(r.mbit_s * 1_000_000, lang)} · {r.path}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
