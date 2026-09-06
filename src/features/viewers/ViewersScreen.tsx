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

import { useEffect, useMemo, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { PlacesTables } from "./PlacesTables";
import { useActiveServer, useServers } from "../servers/store";
import { useT } from "../../shared/i18n";
import { ipc, onViewersUpdate } from "../../shared/ipc";
import type { AppError, LibraryView, Viewer } from "../../shared/contract";
import { LimitDialog } from "./LimitDialog";
import { ViewerRow } from "./ViewerRow";

/** A medium, in the two forms this screen needs it in. */
type Named = { id: string; slug: string; title: string };

/**
 * The media of this server, so the list can name what is being watched and the cap dialog
 * can say which set it applies to.
 *
 * ⚠ **The list, and not a map, because the map was keyed by the wrong thing** (T501). This
 * used to hand back `{ [media.id]: title }`, and the cap dialog then read those entries as
 * `([slug, title]) => ({ slug, title })` — renaming an identifier to a slug and nothing more.
 * Identifiers are `m_<uuid>`; the core builds `{video_dir}/{slug}/master.m3u8`, so the answer
 * was `NoLadderForMedia` every time, the preview stayed empty and the confirm button was
 * disabled for good. Capping a viewer's quality could not be done from this screen at all —
 * FR-060 and FR-061 both.
 *
 * The two readers want different keys: a viewer record names the medium by `media_id`, while
 * a quality set is found by `slug`. Handing over the media themselves lets each take what it
 * needs, and there is nothing left to rename in passing.
 */
function useMedia(serverId: string | null): Named[] {
  const [media, setMedia] = useState<Named[]>([]);

  useEffect(() => {
    if (!serverId) return;
    let alive = true;
    ipc
      .libraryList(serverId)
      .then((view: LibraryView) => {
        if (!alive) return;
        setMedia(view.media.map((m) => ({ id: m.id, slug: m.slug, title: m.title })));
      })
      // A library that will not load is not a reason to hide the viewers: they are still
      // there, and their addresses and speeds are the point. They simply show up under
      // "what they are watching is not known".
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [serverId]);

  return media;
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
  const media = useMedia(serverId);
  // By identifier, which is how a viewer record names what it is watching.
  const titleById = useMemo(() => Object.fromEntries(media.map((m) => [m.id, m.title])), [media]);

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
                mediaTitle={viewer.media_id ? titleById[viewer.media_id] : undefined}
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
          media={media}
          onDone={() => setCapping(null)}
          onCancel={() => setCapping(null)}
        />
      )}
    </section>
  );
}
