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
 *
 * ⚠ **T665 (QA-24B-06) — one refused film does not keep the rest out, and a retry does not put
 * the accepted ones in twice.** The whole loop used to sit inside one `try`: the first refusal
 * (a damaged or missing source is refused before any task exists — `commands/quality.rs`)
 * ended it, the films after it were never put in, and the list kept every film — so pressing
 * start again put the ones already running in a second time. **Owner's decision
 * (2026-09-30):** every film is tried whatever happens to the others; what was refused is
 * listed at the end, each with its reason, with "retry these" — even when every one was
 * refused. The outcome is kept per film: an accepted film leaves the list and is never sent
 * again.
 */

import { useRef, useState } from "react";
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

/** A film the core would not take, and why. */
interface Refused {
  path: string;
  error: AppError;
}

export function BatchScreen() {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.batch;
  const server = useActiveServer();
  // Chosen and not yet put in. A film leaves this list once it has been tried: into the
  // queue if it was taken, into `refused` if it was not.
  const [files, setFiles] = useState<string[]>([]);
  const [refused, setRefused] = useState<Refused[]>([]);
  const [busy, setBusy] = useState(false);
  const [started, setStarted] = useState<number | null>(null);
  // The batch the last press made, so that "retry these" puts the refused films into the
  // same batch — "stop the whole batch" must still reach them.
  const lastBatch = useRef<string | null>(null);

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

  /** Try each of `paths` in turn, whatever happens to the others. */
  const put = async (paths: string[], batchId: string) => {
    if (!server || !isReady(server) || paths.length === 0) return;
    setBusy(true);
    lastBatch.current = batchId;
    const taken: string[] = [];
    const notTaken: Refused[] = [];
    for (const path of paths) {
      try {
        await ipc.qualityMeasureStart({
          path,
          then_build: { server_id: server.id, slug: slugOf(path) },
          batch: { id: batchId, label: filmLabel(path) },
        });
        taken.push(path);
      } catch (e) {
        // This film, and only this one. The next is tried all the same (owner, 2026-09-30).
        notTaken.push({ path, error: toAppError(e) });
      }
    }
    // Everything tried leaves the list of films to put in — the taken ones for good, so a
    // second press cannot send them again; the refused ones to their own list below.
    setFiles((have) => have.filter((p) => !paths.includes(p)));
    setRefused((before) => [...before.filter((r) => !paths.includes(r.path)), ...notTaken]);
    setStarted(taken.length);
    setBusy(false);
  };

  // One identifier for the lot, made here: it is what "stop the whole batch" means, and it
  // has to be the same for every film put in by this press.
  const start = () => put(files, `batch-${Date.now()}`);
  const retry = () =>
    put(
      refused.map((r) => r.path),
      lastBatch.current ?? `batch-${Date.now()}`,
    );

  const noServer = !server || !isReady(server);

  return (
    <div className="panel">
      <h1>{words.title}</h1>
      <p className="hint">{words.explain}</p>

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
        disabled={busy || files.length === 0 || noServer}
        onClick={() => void start()}
      >
        {busy ? words.starting : words.start}
      </button>
      {noServer && (
        <p role="note" data-testid="batch-no-server">
          {words.noServer}
        </p>
      )}

      {started !== null && started > 0 && (
        <p role="status" data-testid="batch-started">
          {fill(words.started, { n: started }, t, lang)}
        </p>
      )}

      {/* What the core would not take, film by film, with its reason — and one press to try
          exactly these again. Shown even when every film was refused: "nothing started" with
          no names is the least useful thing to be told. */}
      {refused.length > 0 && (
        <section className="batch-refused" data-testid="batch-refused">
          <p className="notice__message">
            <strong>{fill(words.refused, { n: refused.length }, t, lang)}</strong>
          </p>
          <ul className="notice__list">
            {/* Each through the common notice: what happened, what to do about it, and the
                particulars — the same three halves an error has everywhere else. */}
            {refused.map(({ path, error }) => (
              <li key={path} data-testid={`batch-refused-${filmLabel(path)}`}>
                <strong>{filmLabel(path)}</strong>
                <ErrorNotice error={error} />
              </li>
            ))}
          </ul>
          <button
            type="button"
            data-testid="batch-retry"
            disabled={busy || noServer}
            onClick={() => void retry()}
          >
            {words.retryThese}
          </button>{" "}
          <button
            type="button"
            className="button-link"
            disabled={busy}
            onClick={() => setRefused([])}
          >
            {words.forgetThese}
          </button>
        </section>
      )}
    </div>
  );
}
