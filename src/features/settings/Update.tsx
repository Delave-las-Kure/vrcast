/**
 * T352 — the update screen (FR-113).
 *
 * **The check happens on a press, never on its own.** An application that reaches for the
 * network when it starts does so on the machine of somebody working offline and on the machine
 * of somebody who would rather it went nowhere; neither of them asked for it. That is why the
 * version and the packaging come from `updateStanding`, which cannot leave this machine, and
 * only `updateCheck` goes anywhere — the rule is in the shape of the two calls rather than in a
 * comment asking the next person to be careful.
 *
 * **The running tasks are shown only where installing stops the application**, which is
 * Windows and nowhere else. There the installer kills the application as its first act, with no
 * event and no delay, so "what happens to what I have running" is exactly the question closing
 * asks — and `tasks_on_close` already answers it per task: resumes, or starts over, and why. It
 * is shown by the component the close dialog uses, because a dialog of its own would be a
 * second answer to one question and the two would drift apart quietly.
 *
 * On Linux nothing is stopped: the plugin rewrites the AppImage or hands the package to
 * `dpkg`, and the running copy carries on with the old code until somebody starts it again.
 * A list of endangered tasks there would be frightening a person about something that is not
 * going to happen. (Read out of `tauri-plugin-updater` 2.10.1: the `process::exit(0)` after
 * installing sits inside `#[cfg(windows)]`.)
 *
 * **What updating costs depends on how this copy was installed**, and the copy knows: the
 * bundler wrote it in. A package asks for an administrator password, an AppImage does not, and
 * a build from the source tree cannot update at all — each said plainly, rather than covered by
 * one careful sentence that fits all three and tells nobody anything.
 */

import { useCallback, useEffect, useState } from "react";

import { CloseConsequences } from "../tasks/CloseConsequences";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type {
  AppError,
  Found,
  InstalledAs,
  TaskOnClose,
  UpdateStanding,
} from "../../shared/contract";

/** What stopping the application costs on this copy, in one sentence or none. */
function warningFor(installed: InstalledAs, words: ReturnType<typeof useT>["ui"]["update"]) {
  switch (installed) {
    case "windows":
      return words.warnWindows;
    case "deb":
    case "rpm":
      return words.warnPackage;
    case "app_image":
      return words.warnAppImage;
    default:
      return null;
  }
}

export function Update() {
  const t = useT();
  const words = t.ui.update;

  const [standing, setStanding] = useState<UpdateStanding | null>(null);
  const [found, setFound] = useState<Found | null>(null);
  const [running, setRunning] = useState<TaskOnClose[]>([]);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  /** Only ever true where the application survives its own installer, which is not Windows. */
  const [installed, setInstalled] = useState(false);
  const [agreed, setAgreed] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .updateStanding()
      .then((got) => {
        if (alive) setStanding(got);
      })
      .catch(() => {
        /* Nothing to show is better than a red panel for a question nobody asked. */
      });
    return () => {
      alive = false;
    };
  }, []);

  const check = useCallback(async () => {
    setChecking(true);
    setError(null);
    try {
      setFound(await ipc.updateCheck());
    } catch (e) {
      setError(e as AppError);
    } finally {
      setChecking(false);
    }
  }, []);

  const install = useCallback(async () => {
    setInstalling(true);
    setError(null);
    try {
      await ipc.updateInstall(true);
      // Reached everywhere except Windows, where the installer stops the application first.
      // Here the new version is on disk and the old one is still running, which is worth
      // saying: otherwise the button goes quiet and nothing appears to have happened.
      setInstalled(true);
      setInstalling(false);
    } catch (e) {
      // Wherever this is reached, the copy on disk is the one that was there before.
      setError(e as AppError);
      setInstalling(false);
    }
  }, []);

  // The list of running tasks is fetched once there is something to install, and not before:
  // it is the answer to a question nobody has asked until then — and only where installing
  // actually stops the application.
  const available = found?.kind === "available" ? found : null;
  const stopsTheApplication = standing?.installed_as === "windows";
  useEffect(() => {
    if (!available || !stopsTheApplication) return;
    let alive = true;
    ipc
      .tasksOnClose()
      .then((tasks) => {
        if (alive) setRunning(tasks);
      })
      .catch(() => {
        if (alive) setRunning([]);
      });
    return () => {
      alive = false;
    };
  }, [available, stopsTheApplication]);

  const packaged = standing !== null && standing.installed_as !== "unpackaged";
  const warning = standing ? warningFor(standing.installed_as, words) : null;
  const canAsk = packaged && standing.configured;

  return (
    <section className="panel__section">
      <h2>{words.title}</h2>

      {standing && (
        <p>
          {words.installed}: <strong>{standing.current}</strong>
        </p>
      )}

      {standing && !packaged && <p className="hint">{words.unpackaged}</p>}
      {packaged && !standing.configured && <p className="hint">{words.notConfigured}</p>}

      {canAsk && (
        <button type="button" onClick={check} disabled={checking || installing}>
          {checking ? words.checking : words.check}
        </button>
      )}

      {found?.kind === "up_to_date" && !checking && <p>{words.upToDate}</p>}

      {available && (
        <div className="notice notice--ok">
          <div className="notice__body">
            <strong className="notice__message">{words.available(available.version)}</strong>
            {available.date && (
              <p>
                {words.published}: {available.date}
              </p>
            )}
            {available.notes && (
              <>
                <h3>{words.notes}</h3>
                <pre className="notes">{available.notes}</pre>
              </>
            )}
          </div>
        </div>
      )}

      {available && stopsTheApplication && <CloseConsequences items={running} />}
      {available && warning && <p className="hint">{warning}</p>}
      {installed && <p data-testid="update-installed">{words.doneRestartLater}</p>}

      {/* Gone once it is installed: the work is done, and the next step is starting the
          application again — not pressing this a second time. */}
      {available && !installed && (
        <>
          <label>
            <input
              type="checkbox"
              checked={agreed}
              onChange={(e) => setAgreed(e.target.checked)}
              disabled={installing}
            />
            {words.agree}
          </label>
          <button type="button" onClick={install} disabled={!agreed || installing}>
            {installing ? words.installing : words.install}
          </button>
        </>
      )}

      {error && <ErrorNotice error={error} />}
    </section>
  );
}
