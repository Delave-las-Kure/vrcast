/**
 * T293 — ведёт ли домен на этот сервер (FR-137, FR-138, FR-140).
 *
 * **Спрашивается заново, а не один раз.** Доменная запись расходится по сети минутами, и
 * человек, только что заведший её у регистратора, увидит здесь «ещё нет» — на что правильный
 * ответ спросить снова, а не начать развёртывание и не закрыть экран.
 *
 * И отказ здесь никогда не звучит как ошибка разрешения имени. Тому, кто впервые купил
 * сервер, `NXDOMAIN` не говорит ничего; ему нужны тип записи, её точное имя и точное
 * значение — а если запись уже есть, то ещё и куда она ведёт сейчас, потому что чаще всего
 * это остаток от прошлой жизни домена.
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
  /** Наверх — чтобы экран развёртывания знал, можно ли начинать. */
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

  // Спрашивается при открытии и при смене выбора про IPv6: тот же домен при «оставить» и
  // при «отключить» — два разных вердикта, и показывать вчерашний было бы хуже, чем не
  // показывать никакого.
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
          {/* Что пойти и сделать — кодом со значениями; формулировка живёт в словаре. */}
          <p>{answer.advice ? renderDetail(answer.advice, t, lang) : words.domainNotPointed}</p>

          {/* Куда ведёт сейчас. Показано отдельно от совета: человек сверяет это со
              страницей своего регистратора глазами. */}
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
