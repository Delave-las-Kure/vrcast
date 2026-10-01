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
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { ipc, onViewersUpdate } from "../../shared/ipc";
import type { AppError, LibraryView, Viewer, ViewersUpdateEvent } from "../../shared/contract";
import { LimitDialog } from "./LimitDialog";
import { ViewerRow } from "./ViewerRow";

/**
 * This machine's clock, ticking once a second while `running` — so the age of a list that is
 * no longer being kept up to date goes on growing on screen (T664). Still otherwise: a
 * current list needs no clock, and a timer nobody reads is a timer for nothing.
 */
function useNow(running: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);
  return now;
}

/** "The list below is the last one received, 42 s ago." — or that there was none. */
function ageLine(
  asOf: string | null,
  now: number,
  words: Catalogue["ui"]["viewers"],
  t: Catalogue,
  lang: Lang,
): string {
  const at = asOf ? Date.parse(asOf) : NaN;
  if (Number.isNaN(at)) return words.staleNever;
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  const age =
    seconds < 120
      ? fill(words.ageSeconds, { n: seconds }, t, lang)
      : fill(words.ageMinutes, { n: Math.floor(seconds / 60) }, t, lang);
  return fill(words.staleAge, { age }, t, lang);
}

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
  const { lang } = useLang();
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
  // And the set a viewer is watching, by the same identifier — for the cap dialog (T668).
  const slugById = useMemo(() => Object.fromEntries(media.map((m) => [m.id, m.slug])), [media]);

  const [viewers, setViewers] = useState<Viewer[] | null>(null);
  // Where the watching stands (T664). Kept beside the list rather than folded into it: the
  // list stays on screen while the connection is being got back, and what changes is
  // whether it may be read as "now".
  const [watch, setWatch] = useState<Pick<ViewersUpdateEvent, "watch" | "as_of" | "attempt">>({
    watch: "watching",
    as_of: null,
    attempt: 0,
  });
  // Bumped by "start again" after the watching has given up: the effect below runs afresh.
  const [restarts, setRestarts] = useState(0);
  // Whom the person is about to cap, if anybody, and what they are watching. The dialogue is
  // opened from the row rather than from a screen of its own: capping is something done
  // **to a viewer you are looking at**, and making somebody go elsewhere and retype an
  // address would be three actions where SC-006 allows three altogether. The medium goes
  // with the address (T668, QA-24B-09): the dialog used to open on the first film of the
  // catalogue, whatever the viewer was watching.
  const [capping, setCapping] = useState<{ ip: string; slug: string | null } | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    if (!serverId) return;
    let alive = true;
    setError(null);
    setViewers(null);
    setWatch({ watch: "watching", as_of: null, attempt: 0 });

    const unlisten = onViewersUpdate((update) => {
      if (!alive || update.server_id !== serverId) return;
      setViewers(update.active);
      setWatch({
        watch: update.watch ?? "watching",
        as_of: update.as_of ?? null,
        attempt: update.attempt ?? 0,
      });
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
  }, [serverId, restarts]);

  const stale = watch.watch !== "watching";
  // The age of a list nobody is keeping up to date grows by itself, with no event to say so.
  const now = useNow(stale);

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

      {watch.watch === "reconnecting" && (
        <div className="notice notice--warning" role="status" data-testid="viewers-reconnecting">
          <div className="notice__body">
            <strong className="notice__message">{words.reconnecting}</strong>
            {watch.attempt > 0 && (
              <span className="notice__hint">
                {" "}
                {fill(words.reconnectingTry, { n: watch.attempt }, t, lang)}
              </span>
            )}
            <p className="notice__hint" id="viewers-age" data-testid="viewers-age">
              {ageLine(watch.as_of, now, words, t, lang)}
            </p>
          </div>
        </div>
      )}

      {watch.watch === "stopped" && (
        <div className="notice notice--error" role="alert" data-testid="viewers-stopped">
          <div className="notice__body">
            <strong className="notice__message">{words.stopped}</strong>
            <p className="notice__hint" id="viewers-age" data-testid="viewers-age">
              {ageLine(watch.as_of, now, words, t, lang)}
            </p>
            <button type="button" onClick={() => setRestarts((n) => n + 1)}>
              {words.restart}
            </button>
          </div>
        </div>
      )}

      {viewers === null && !error && <p className="hint">{words.starting}</p>}

      {viewers !== null && viewers.length === 0 && !stale && (
        // Not an error and not a blank screen: nobody watching is the ordinary state most
        // of the time, and it must not look like something failed to load.
        <p className="hint" role="status">
          {words.nobody}
        </p>
      )}

      {viewers !== null && viewers.length > 0 && (
        <table
          className={stale ? "viewers viewers--stale" : "viewers"}
          data-testid="viewers-table"
          data-stale={stale ? "true" : "false"}
          aria-describedby={stale ? "viewers-age" : undefined}
          // Faded while it is not current: the same rows, readable, and plainly not "now".
          style={stale ? { opacity: 0.55 } : undefined}
        >
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
                onLimit={() =>
                  setCapping({
                    ip: viewer.ip,
                    slug: viewer.media_id ? (slugById[viewer.media_id] ?? null) : null,
                  })
                }
                limitLabel={t.ui.limits.title}
              />
            ))}
          </tbody>
        </table>
      )}

      {capping && serverId && (
        <LimitDialog
          key={`${capping.ip}/${capping.slug ?? ""}`}
          serverId={serverId}
          ip={capping.ip}
          initialSlug={capping.slug}
          media={media}
          onDone={() => setCapping(null)}
          onCancel={() => setCapping(null)}
        />
      )}
    </section>
  );
}
