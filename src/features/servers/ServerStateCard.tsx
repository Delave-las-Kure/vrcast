/**
 * T294 — what kind of server this is, right on its card (FR-120, FR-128, FR-130, FR-132).
 *
 * Five states, and each sends a person somewhere different. A general "something is wrong"
 * sends them nowhere:
 *
 * - **clean** — offer to deploy;
 * - **unfinished** — offer to finish. This is our own interrupted work, and calling it
 *   somebody else's server means refusing to finish what we started;
 * - **ours** — the server side's version beside the application's. Apart they mean nothing:
 *   "server side 1" is a number until it is said which one the application deploys;
 * - **newer than we understand** — warn and change nothing (FR-130). An older application
 *   writing files where a newer layout does not keep them is a way to break a working server
 *   quietly;
 * - **somebody else's** — say **what exactly** was recognised (FR-132). "A foreign
 *   configuration" is not something anybody can go and deal with.
 */

import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import { UpgradeDialog } from "./UpgradeDialog";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type { AppError, ServerState } from "../../shared/contract";

export function ServerStateCard({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.serverState;

  const [state, setState] = useState<ServerState | null>(null);
  const [asking, setAsking] = useState(true);
  const [error, setError] = useState<AppError | null>(null);
  const [upgrading, setUpgrading] = useState(false);

  useEffect(() => {
    let alive = true;
    setAsking(true);
    ipc
      .serverDetect(serverId)
      .then((got) => {
        if (alive) setState(got);
      })
      .catch((e: AppError) => {
        // A server that did not answer is not a fault in the application. The reason is
        // shown, and the state is not pretended to be known.
        if (alive) setError(e);
      })
      .finally(() => {
        if (alive) setAsking(false);
      });
    return () => {
      alive = false;
    };
  }, [serverId, upgrading]);

  if (asking) return <p>{words.asking}</p>;
  if (error) return <ErrorNotice error={error} />;
  if (!state) return null;

  if (upgrading) {
    return (
      <UpgradeDialog
        serverId={serverId}
        onDone={() => setUpgrading(false)}
        onCancel={() => setUpgrading(false)}
      />
    );
  }

  return (
    <div aria-label={words.title}>
      {state.kind === "Clean" && (
        <>
          <p>{words.clean}</p>
          <Link to={`/deploy?server=${encodeURIComponent(serverId)}`}>{words.deployIt}</Link>
        </>
      )}

      {state.kind === "Unfinished" && (
        <>
          <p>{words.unfinished}</p>
          <Link to={`/deploy?server=${encodeURIComponent(serverId)}`}>{words.finishIt}</Link>
        </>
      )}

      {state.kind === "Managed" && (
        <>
          {/* Together, not apart. */}
          <p>{words.versions(state.server_version ?? 0, state.app_expects)}</p>
          {state.compat === "TooNew" && <p>{words.tooNew}</p>}
          {(state.upgrade_available || state.compat === "NeedsUpgrade") && (
            <button type="button" onClick={() => setUpgrading(true)}>
              {words.updateIt}
            </button>
          )}
        </>
      )}

      {state.kind === "Foreign" && (
        <>
          <p>{words.foreign}</p>
          {/* What exactly was found. Without it the refusal attaches to nothing. */}
          {state.foreign_reason !== null && <small>{JSON.stringify(state.foreign_reason)}</small>}
        </>
      )}

      {state.kind === "Unreachable" && <p>{words.unreachable}</p>}
    </div>
  );
}
