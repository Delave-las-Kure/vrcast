/**
 * T293 — does the domain lead to this server (FR-137, FR-138, FR-140).
 *
 * **Asked again, not once.** A DNS record takes minutes to travel, and somebody who has just
 * created one at their registrar will see "not yet" here — to which the right answer is to ask
 * again, not to start the deployment and not to close the screen.
 *
 * And a refusal here never sounds like a name-resolution error. To somebody who has bought a
 * server for the first time, `NXDOMAIN` says nothing; what they need is the record type, its
 * exact name and its exact value — and, if a record already exists, where it leads now, because
 * most often that is a leftover from the domain's previous life.
 */

import { useCallback, useEffect, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import { ipc } from "../../shared/ipc";
import type { AppError, DomainAnswer, Ipv6Choice } from "../../shared/contract";

export function DomainCheck({
  serverId,
  ipv6,
  onAnswer,
}: {
  serverId: string;
  ipv6: Ipv6Choice;
  /** Upwards, so the deployment screen knows whether it may begin. */
  onAnswer?: (answer: DomainAnswer) => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.deploy;

  const [answer, setAnswer] = useState<DomainAnswer | null>(null);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const ask = useCallback(() => {
    setAsking(true);
    setError(null);
    ipc
      .dnsCheck(serverId, ipv6)
      .then((got) => {
        setAnswer(got);
        onAnswer?.(got);
      })
      .catch((e: AppError) => setError(e))
      .finally(() => setAsking(false));
  }, [serverId, ipv6, onAnswer]);

  // Asked on opening and whenever the IPv6 choice changes: the same domain gives two
  // different verdicts under "keep" and under "turn off", and showing yesterday's would be
  // worse than showing none.
  useEffect(ask, [ask]);

  const ok = answer !== null && answer.advice === null;

  return (
    <section aria-label={words.domainTitle}>
      <h3>{words.domainTitle}</h3>

      {asking && <p>{words.domainAsking}</p>}
      {error && <ErrorNotice error={error} />}

      {answer && ok && <p>{words.domainOk}</p>}

      {answer && !ok && (
        <>
          {/* What to go and do — as a code with values; the wording lives in the catalogue. */}
          <p>{answer.advice ? renderDetail(answer.advice, t, lang) : words.domainNotPointed}</p>

          {/* Where it leads now. Shown apart from the advice: a person compares this with
              their registrar's page by eye. */}
          {(answer.a.length > 0 || answer.aaaa.length > 0) && (
            <dl>
              {answer.a.length > 0 && (
                <>
                  <dt>A</dt>
                  <dd>{answer.a.join(", ")}</dd>
                </>
              )}
              {answer.aaaa.length > 0 && (
                <>
                  <dt>AAAA</dt>
                  <dd>{answer.aaaa.join(", ")}</dd>
                </>
              )}
            </dl>
          )}

          <p>{words.domainSpreadsSlowly}</p>
          <button type="button" onClick={ask} disabled={asking}>
            {words.domainAskAgain}
          </button>
        </>
      )}
    </section>
  );
}
