/**
 * What the serving wrote down over a stretch of time (FR-071).
 *
 * **A long request is ordinary in itself**, and that is said here in words rather than hidden
 * inside a threshold. It is a long range fetch; on a healthy server most of them are. Marked
 * are only the ones that delivered almost nothing while taking that long — otherwise the
 * screen goes red on a working machine, and after the second time people stop looking at it.
 *
 * **Nothing about seeking is read off the mix of answers** (T706, QA-26 №11). It used to be:
 * "206s should be the majority, or seeking does not work". A quality set is fetched as whole
 * small segments, each rightly answered 200, so every healthy set was declared broken. The
 * question belongs to single mp4 files, and is asked of them directly with a range request
 * under the server's state.
 *
 * **Hitting the line limit is said out loud.** A summary that quietly covered a quarter of
 * what was asked for is answering a different question from the one it was given.
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
      {/* T706 (QA-26 №11): nothing about seeking is concluded from this mix any more. A
          quality set is fetched in whole small segments, and 200 is the right answer to each
          of them — so a log of a healthy set "proved" seeking broken. Whether pieces of a
          single mp4 are handed out is asked separately, with a range request, under the
          server's state (the delivery reading). */}

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
                {Math.round(r.seconds)} {words.unitSeconds} ·{" "}
                {formatBitrate(r.mbit_s * 1_000_000, lang)} · {r.path}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
