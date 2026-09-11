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

import { useCallback, useEffect, useRef, useState } from "react";

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
  // What `BitratePeaks` found about the chosen file, if anything (T500). Without it, every
  // stalling viewer falls into the general "not enough channel" verdict — `Cause::TheFileItself`
  // and `Cause::ThePlayer` both require `Some(file)` to be reached at all.
  const [fileShape, setFileShape] = useState<{
    average_mbit: number;
    peak_10s_mbit: number;
  } | null>(null);
  /**
   * T594 — a per-request generation token, the same pattern `DomainCheck`'s `genRef`
   * already uses (T592), and `LadderScreen`'s (T587) and `UploadScreen`'s `uploadGenRef`
   * (T582) before it. `ask` is rebuilt whenever `minutes` changes and the effect below
   * re-runs it, but nothing used to stop an older, slower call from overwriting what a
   * newer one already showed — a person who switches from 120 minutes to 10 right after
   * opening the screen could see the 120-minute figures win the race and sit there under
   * a "10 minutes" label.
   */
  const genRef = useRef(0);

  const ask = useCallback(async () => {
    const gen = ++genRef.current;
    setAsking(true);
    setError(null);
    try {
      // One at a time rather than all at once: there is one connection, and three questions
      // asked together take three channels out of eight — two of which the viewer watching
      // already holds (R-04). The stall reading also measures live load for five seconds, and
      // measuring it while our own questions run alongside is measuring ourselves.
      const health = await ipc.diagHealth(serverId);
      if (gen !== genRef.current) return;
      setHealth(health);

      const logs = await ipc.diagLogs(serverId, minutes);
      if (gen !== genRef.current) return;
      setLogs(logs);

      const stalls = await ipc.diagExplainStalls(serverId, minutes, fileShape ?? undefined);
      if (gen !== genRef.current) return;
      setStalls(stalls);
    } catch (e) {
      if (gen === genRef.current) setError(e as AppError);
    } finally {
      // Checked here too: a stale call finishing after a newer one is still going would
      // otherwise clear `asking` early, and a person would see "done" while the current,
      // actually-relevant request is still in flight.
      if (gen === genRef.current) setAsking(false);
    }
  }, [serverId, minutes, fileShape]);

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

      <BitratePeaks onMeasured={setFileShape} />
    </div>
  );
}
