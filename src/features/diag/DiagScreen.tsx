/**
 * T317 — the diagnostics screen (FR-070, FR-071, FR-072).
 *
 * **The order of the sections is a method, not a layout.** First the state of the server:
 * asleep or working. Then the log: what the serving has actually been doing. Then the reading
 * of the stalls: why it stops for this particular viewer. And the file last, on a tab of its
 * own, because that is where people get to last. The order reversed is an evening spent
 * re-encoding a film for the sake of somebody else's Wi-Fi.
 *
 * **"Could not tell" is a state of its own, not an empty screen.** Emptiness is read as "all
 * is well" or as the application being broken; both are untrue, and the difference between
 * them is the difference between going to fix a server and asking once more.
 *
 * Nothing here changes the server. All four questions are read-only, which is why they can be
 * asked of somebody else's machine and of one newer than this application: those are exactly
 * the machines a person needs to look at.
 */

import { useCallback, useEffect, useState } from "react";

import { BitratePeaks } from "./BitratePeaks";
import { HealthPanel } from "./HealthPanel";
import { LogsPanel } from "./LogsPanel";
import { StallsPanel } from "./StallsPanel";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type { AppError, Health, Logs, Stalls } from "../../shared/contract";

/** How many minutes of log to ask for by default. */
const DEFAULT_MINUTES = 30;

const PERIODS = [10, 30, 60, 120];

export function DiagScreen({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.diag;

  const [minutes, setMinutes] = useState(DEFAULT_MINUTES);
  const [health, setHealth] = useState<Health | null>(null);
  const [logs, setLogs] = useState<Logs | null>(null);
  const [stalls, setStalls] = useState<Stalls | null>(null);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const ask = useCallback(async () => {
    setAsking(true);
    setError(null);
    try {
      // One at a time rather than all at once: there is one connection, and three questions
      // asked together take three channels out of eight — two of which the viewer watching
      // already holds (R-04). The stall reading also measures live load for five seconds, and
      // measuring it while our own questions run alongside is measuring ourselves.
      setHealth(await ipc.diagHealth(serverId));
      setLogs(await ipc.diagLogs(serverId, minutes));
      setStalls(await ipc.diagExplainStalls(serverId, minutes));
    } catch (e) {
      setError(e as AppError);
    } finally {
      setAsking(false);
    }
  }, [serverId, minutes]);

  useEffect(() => {
    void ask();
  }, [ask]);

  const nothingCameBack = !asking && !error && health === null;

  return (
    <div className="diag-screen">
      <h2>{words.title}</h2>

      <div className="diag-controls">
        <label>
          {words.period}{" "}
          <select
            value={minutes}
            onChange={(e) => setMinutes(Number(e.target.value))}
            data-testid="diag-period"
          >
            {PERIODS.map((m) => (
              <option key={m} value={m}>
                {m} {words.minutes}
              </option>
            ))}
          </select>
        </label>
        <button type="button" onClick={() => void ask()} disabled={asking}>
          {words.refresh}
        </button>
      </div>

      {asking && <p data-testid="diag-asking">{words.asking}</p>}
      {error && <ErrorNotice error={error} />}
      {nothingCameBack && <p data-testid="diag-nothing">{words.notDetermined}</p>}

      {health && <HealthPanel health={health} />}
      {logs && <LogsPanel logs={logs} />}
      {stalls && <StallsPanel stalls={stalls} />}

      <BitratePeaks />
    </div>
  );
}
