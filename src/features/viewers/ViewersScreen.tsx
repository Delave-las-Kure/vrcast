/**
 * T174 — who is watching right now.
 *
 * **The list is not asked for.** Watching is switched on when the screen opens and off when
 * it closes; between those the core sends the list as it changes (FR-054). Asking again and
 * again for something that moves every few seconds is what SC-009 exists to prevent, and it
 * would double the traffic to the server for nothing.
 *
 * Switching off on the way out is not tidiness either: watching holds two of the server's
 * eight channels for as long as it runs (R-04), and a screen that forgot to let go would
 * quietly take them out of everything else.
 */

import { useEffect, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { PlacesTables } from "./PlacesTables";
import { useActiveServer, useServers } from "../servers/store";
import { useT } from "../../shared/i18n";
import { ipc, onViewersUpdate } from "../../shared/ipc";
import type { AppError, LibraryView, Viewer } from "../../shared/contract";
import { LimitDialog } from "./LimitDialog";
import { ViewerRow } from "./ViewerRow";

/** What the library calls a medium, so the list can name what is being watched. */
function useMediaTitles(serverId: string | null): Record<string, string> {
  const [titles, setTitles] = useState<Record<string, string>>({});

  useEffect(() => {
    if (!serverId) return;
    let alive = true;
    ipc
      .libraryList(serverId)
      .then((view: LibraryView) => {
        if (!alive) return;
        const next: Record<string, string> = {};
        for (const media of view.media) next[media.id] = media.title;
        setTitles(next);
      })
      // A library that will not load is not a reason to hide the viewers: they are still
      // there, and their addresses and speeds are the point. They simply show up under
      // "what they are watching is not known".
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [serverId]);

  return titles;
}

export function ViewersScreen() {
  const t = useT();
  const words = t.ui.viewers;
  const server = useActiveServer();
  const serverId = server?.id ?? null;
  const reloadServers = useServers((s) => s.reload);
  const serversLoading = useServers((s) => s.loading);

  // The list of servers is loaded here rather than assumed to be loaded already. This
  // screen can be the first one opened — a person may come straight back to where they
  // left off — and without this it would say "choose a server" while one is chosen.
  useEffect(() => {
    void reloadServers();
  }, [reloadServers]);
  const titles = useMediaTitles(serverId);

  const [viewers, setViewers] = useState<Viewer[] | null>(null);
  // Whom the person is about to cap, if anybody. The dialogue is opened from the row
  // rather than from a screen of its own: capping is something done **to a viewer you
  // are looking at**, and making somebody go elsewhere and retype an address would be
  // three actions where SC-006 allows three altogether.
  const [capping, setCapping] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    if (!serverId) return;
    let alive = true;
    setError(null);
    setViewers(null);

    const unlisten = onViewersUpdate((update) => {
      if (alive && update.server_id === serverId) setViewers(update.active);
    });

    ipc.viewersWatchStart(serverId).catch((e: AppError) => {
      if (alive) setError(e);
    });

    return () => {
      alive = false;
      // Both, and in this order: stop listening, then let the channels go. Leaving the
      // watching on would hold two of the server's eight channels for a screen nobody is
      // looking at.
      void unlisten.then((off) => off());
      void ipc.viewersWatchStop().catch(() => undefined);
    };
  }, [serverId]);

  if (!server) {
    return (
      <section className="screen">
        <h1>{t.ui.sections.viewers}</h1>
        <p className="hint">{serversLoading ? t.ui.common.loading : words.noServer}</p>
      </section>
    );
  }

  return (
    <section className="screen">
      <h1>{t.ui.sections.viewers}</h1>
      <p className="hint">{words.explain}</p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {/*
        The tables the countries and cities on this screen come from. Silent while they are
        there and current, which is most of the time.
      */}
      <PlacesTables />

      {viewers === null && !error && <p className="hint">{words.starting}</p>}

      {viewers !== null && viewers.length === 0 && (
        // Not an error and not a blank screen: nobody watching is the ordinary state most
        // of the time, and it must not look like something failed to load.
        <p className="hint" role="status">
          {words.nobody}
        </p>
      )}

      {viewers !== null && viewers.length > 0 && (
        <table className="viewers">
          <thead>
            <tr>
              <th>{words.columnAddress}</th>
              <th>{words.columnPlace}</th>
              <th>{words.columnWatching}</th>
              <th>{words.columnSpeed}</th>
              <th>{words.columnFor}</th>
              <th>{words.columnState}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {viewers.map((viewer) => (
              <ViewerRow
                key={viewer.ip}
                viewer={viewer}
                mediaTitle={viewer.media_id ? titles[viewer.media_id] : undefined}
                onLimit={() => setCapping(viewer.ip)}
                limitLabel={t.ui.limits.title}
              />
            ))}
          </tbody>
        </table>
      )}

      {capping && serverId && (
        <LimitDialog
          serverId={serverId}
          ip={capping}
          media={Object.entries(titles).map(([slug, title]) => ({ slug, title }))}
          onDone={() => setCapping(null)}
          onCancel={() => setCapping(null)}
        />
      )}
    </section>
  );
}
