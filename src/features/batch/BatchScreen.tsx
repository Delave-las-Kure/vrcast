/**
 * T443 — a season goes in, a season comes out.
 *
 * **The owner's own words**: "the user picks videos (several can be queued). Each video's
 * ladder is measured. Then the mp4s and m3u8s are assembled from those ladders." Until now
 * every route through this application took one file: choose it, wait, choose the next. A
 * season of twelve meant sitting down twelve times, hours apart, for a decision the
 * application had already worked out.
 *
 * **The chain is not here.** This screen puts the measurements in and stops. What happens
 * between "these are the rungs" and "send them" is decided in the core (T438), because by
 * then the window may be shut or in the tray — and a decision taken by a closed window is
 * taken by nobody. A screen that drove the chain would work perfectly while somebody watched
 * it and do nothing at all the moment they stopped, which is exactly when a batch is left to
 * run.
 *
 * **What it does not do.** It does not offer to edit the rungs of twelve films. That is what
 * the ladder screen is for, one film at a time, and a batch that stopped to ask about each
 * one would be twelve sittings again wearing a different hat. The core stops on an objection
 * and says so on the task (T439); the rest goes through.
 */

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import type { AppError } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { ErrorNotice } from "../shared/ErrorNotice";
import { filmLabel, slugOf } from "../shared/names";
import { isReady, useActiveServer } from "../servers/store";

/** What every route through this application already accepts. */
const VIDEO = { name: "video", extensions: ["mp4", "mkv", "mov", "webm", "m4v", "avi", "ts"] };

export function BatchScreen() {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.batch;
  const server = useActiveServer();
  const [files, setFiles] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [started, setStarted] = useState<number | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  const pick = async () => {
    // `multiple: true`, which is the whole point of this screen: the three other dialogues in
    // this application each set it to false by hand.
    const chosen = await open({ multiple: true, directory: false, filters: [VIDEO] });
    if (!Array.isArray(chosen)) return;
    // Added to what is there rather than replacing it: a season split across two folders is
    // two visits to the dialogue, and the second must not throw away the first.
    setFiles((have) => [...new Set([...have, ...chosen])]);
    setStarted(null);
  };

  const start = async () => {
    if (!server || !isReady(server) || files.length === 0) return;
    setBusy(true);
    setError(null);
    // One identifier for the lot, made here: it is what "stop the whole batch" means, and it
    // has to be the same for every film put in by this press.
    const batchId = `batch-${Date.now()}`;
    let put = 0;
    try {
      for (const path of files) {
        await ipc.qualityMeasureStart({
          path,
          then_build: { server_id: server.id, slug: slugOf(path) },
          batch: { id: batchId, label: filmLabel(path) },
        });
        put += 1;
      }
      setFiles([]);
    } catch (e) {
      // What went in stays in. A batch that fails on the eighth film has seven measurements
      // running, and telling somebody nothing started would send them to cancel work that is
      // already under way.
      setError(toAppError(e));
    } finally {
      setStarted(put);
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <h1>{words.title}</h1>
      <p className="hint">{words.explain}</p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <button type="button" onClick={() => void pick()} disabled={busy}>
        {words.pick}
      </button>

      {files.length > 0 && (
        <>
          <ul data-testid="batch-files">
            {files.map((path) => (
              <li key={path}>
                {filmLabel(path)}{" "}
                <button
                  type="button"
                  className="button-link"
                  onClick={() => setFiles((have) => have.filter((it) => it !== path))}
                  aria-label={fill(words.dropOne, { film: filmLabel(path) }, t, lang)}
                >
                  {words.drop}
                </button>
              </li>
            ))}
          </ul>
          <p className="hint" data-testid="batch-count">
            {fill(words.count, { n: files.length }, t, lang)}
          </p>
        </>
      )}

      {/* A button that does nothing teaches people the application is broken, so the reason
          it cannot be pressed is written beside it rather than left to be guessed at. */}
      <button
        type="button"
        data-testid="batch-start"
        disabled={busy || files.length === 0 || !server || !isReady(server)}
        onClick={() => void start()}
      >
        {busy ? words.starting : words.start}
      </button>
      {(!server || !isReady(server)) && (
        <p role="note" data-testid="batch-no-server">
          {words.noServer}
        </p>
      )}

      {started !== null && started > 0 && (
        <p role="status" data-testid="batch-started">
          {fill(words.started, { n: started }, t, lang)}
        </p>
      )}
    </div>
  );
}
