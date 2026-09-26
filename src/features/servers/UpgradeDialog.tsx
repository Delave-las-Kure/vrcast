/**
 * T295 — upgrading the server side (FR-129, FR-131, FR-133).
 *
 * **A list of changes, not "an update is available".** A person is agreeing to a change to a
 * machine that is not theirs to begin with and that holds their films; they have a right to
 * know what change.
 *
 * And beside it, what will be copied aside before the first edit — because the promise that it
 * can be put back is worth exactly as much as a person knows about it. The video directory and
 * the manifest are not on that list and must not be: that is their work, not our
 * configuration, and a rollback that restored the manifest would undo everything uploaded
 * since.
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
  /**
   * T593 — its own flag, not `running`: `running` holds the id of the real task once
   * `serverUpgradeRun` has answered, and the effect above compares `event.id !== running`
   * against it — it means "a task is now actually going". `starting` means something
   * narrower and earlier: "my own click has been sent, its answer has not arrived yet".
   * Conflating the two would mean the button re-enables (or the handler re-runs) during
   * the exact window between a click and the response, which is precisely the gap a
   * second unasked upgrade slips through.
   */
  const [starting, setStarting] = useState(false);
  /**
   * T611 — the rollback asks first. It used to go at the first click, and what it does is
   * narrower than its name: it puts the copied files back and nothing else. A person reaching
   * for "put it back as it was" after something went wrong is owed the list of what it will
   * not put back — files the run created, live state, the limit rules — before it runs.
   */
  const [confirmingRollback, setConfirmingRollback] = useState(false);
  const [rollingBack, setRollingBack] = useState(false);
  const [rolledBack, setRolledBack] = useState(false);

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
          {/* Named one by one. "A backup will be made" is not a promise anybody can check;
              this list is. */}
          <ul>
            {plan.backing_up.map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
          <p>{words.keepsVideosAndCatalogue}</p>

          <button
            type="button"
            disabled={running !== null || toDo.length === 0 || starting}
            onClick={() => {
              // T593 — same synchronous refusal as DeployScreen's `start`: a second click
              // that lands before this one's answer must not send its own
              // `serverUpgradeRun`, an unasked-for second upgrade on the server.
              if (starting) return;
              setStarting(true);
              setError(null);
              ipc
                .serverUpgradeRun(serverId, true)
                .then(setRunning)
                .catch((e: AppError) => setError(e))
                .finally(() => setStarting(false));
            }}
          >
            {words.agreeAndUpgrade}
          </button>
          <button type="button" onClick={onCancel} disabled={running !== null}>
            {words.cancel}
          </button>

          {/* Rolling back stands beside upgrading rather than hiding: people reach for it
              just after an upgrade has gone wrong, and hunting for it at that moment is one
              thing too many. It asks before it runs (T611): what comes back, and what does
              not. */}
          <button
            type="button"
            disabled={running !== null || rollingBack || confirmingRollback}
            onClick={() => {
              setError(null);
              setRolledBack(false);
              setConfirmingRollback(true);
            }}
          >
            {words.rollBack}
          </button>

          {confirmingRollback && (
            <div role="dialog" aria-label={words.rollBackTitle}>
              <h4>{words.rollBackTitle}</h4>
              <p>{words.rollBackReturns}</p>
              <p>{words.rollBackKeeps}</p>
              <button
                type="button"
                disabled={rollingBack}
                onClick={() => {
                  // Refused in the handler as well as by `disabled`, as the other buttons
                  // here are (T593): a second click must not send a second rollback.
                  if (rollingBack) return;
                  setRollingBack(true);
                  setError(null);
                  ipc
                    .serverRollback(serverId)
                    .then(() => {
                      setConfirmingRollback(false);
                      setRolledBack(true);
                    })
                    .catch((e: AppError) => {
                      setConfirmingRollback(false);
                      setError(e);
                    })
                    .finally(() => setRollingBack(false));
                }}
              >
                {words.rollBackConfirm}
              </button>
              <button
                type="button"
                disabled={rollingBack}
                onClick={() => setConfirmingRollback(false)}
              >
                {words.cancel}
              </button>
            </div>
          )}
          {rolledBack && <p>{words.rollBackDone}</p>}
        </>
      )}
    </section>
  );
}
