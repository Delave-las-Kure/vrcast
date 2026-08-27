/**
 * T295 — обновить серверную часть (FR-129, FR-131, FR-133).
 *
 * **Перечень изменений, а не «доступно обновление».** Человек подтверждает изменение машины,
 * которая для него чужая и на которой лежат его фильмы; он вправе знать, чего именно.
 *
 * И рядом — что будет скопировано в сторону до первой правки, потому что обещание «можно
 * вернуть как было» стоит ровно столько, сколько человек про него знает. Каталог видео и
 * опись в этот список не входят и входить не должны: это его работа, а не наша настройка, и
 * откат, вернувший опись, отменил бы всё залитое с тех пор.
 */

import { useEffect, useState } from "react";

import { StepList } from "../deploy/StepList";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc, onTaskDone } from "../../shared/ipc";
import type { AppError, UpgradePlan } from "../../shared/contract";

export function UpgradeDialog({
  serverId,
  onDone,
  onCancel,
}: {
  serverId: string;
  onDone?: () => void;
  onCancel?: () => void;
}) {
  const t = useT();
  const words = t.ui.upgrade;

  const [plan, setPlan] = useState<UpgradePlan | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .serverUpgradePlan(serverId)
      .then((got) => {
        if (alive) setPlan(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, [serverId]);

  useEffect(() => {
    if (!running) return;
    let alive = true;
    const finish = onTaskDone((event) => {
      if (!alive || event.id !== running) return;
      setRunning(null);
      if (event.error) setError(event.error);
      else onDone?.();
    });
    return () => {
      alive = false;
      void finish.then((off) => off());
    };
  }, [running, onDone]);

  const toDo = plan?.steps.filter((s) => s.status === "NotApplied") ?? [];

  return (
    <section aria-label={words.title}>
      <h3>{words.title}</h3>

      {error && <ErrorNotice error={error} />}

      {plan && (
        <>
          <p>{words.fromTo(plan.from, plan.to)}</p>

          {toDo.length === 0 ? (
            <p>{words.nothingToDo}</p>
          ) : (
            <>
              <h4>{words.willChange}</h4>
              <StepList steps={toDo} />
            </>
          )}

          <h4>{words.willKeep}</h4>
          {/* Названы поимённо. «Будет сделана резервная копия» — это не обещание, которое
              можно проверить, а этот список можно. */}
          <ul>
            {plan.backing_up.map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
          <p>{words.keepsVideosAndCatalogue}</p>

          <button
            type="button"
            disabled={running !== null || toDo.length === 0}
            onClick={() => {
              setError(null);
              ipc
                .serverUpgradeRun(serverId, true)
                .then(setRunning)
                .catch((e: AppError) => setError(e));
            }}
          >
            {words.agreeAndUpgrade}
          </button>
          <button type="button" onClick={onCancel} disabled={running !== null}>
            {words.cancel}
          </button>

          {/* Возврат стоит рядом с обновлением, а не прячется: к нему тянутся тогда, когда
              обновление только что пошло не так, и искать его в этот момент — лишнее. */}
          <button
            type="button"
            disabled={running !== null}
            onClick={() => {
              setError(null);
              ipc.serverRollback(serverId).catch((e: AppError) => setError(e));
            }}
          >
            {words.rollBack}
          </button>
        </>
      )}
    </section>
  );
}
