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

          {/* Rolling back stands beside upgrading rather than hiding: people reach for it
              just after an upgrade has gone wrong, and hunting for it at that moment is one
              thing too many. */}
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
