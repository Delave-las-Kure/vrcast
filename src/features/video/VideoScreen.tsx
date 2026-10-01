/**
 * T673 — the «Video» screen: every video from its file to its link, in one place.
 *
 * Add files, look at the plan, press «Start»; from there each video goes through measuring,
 * encoding, sending, cutting and checking by itself, and its card shows which stage it is at.
 * It replaces four screens (preparation, qualities, batch, upload) between which a person
 * used to carry each video by hand.
 *
 * **The list is asked for once, then only listened to.** `videoList` on the way in, after
 * that `video:update` carries the whole video every time. After a restart the core still
 * has every video and the stage it was at, so the same one call puts the screen back.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Link, useSearchParams } from "react-router-dom";

import type { AppError, VideoRefusal, VideoView } from "../../shared/contract";
import { ipc, onVideoUpdate, toAppError } from "../../shared/ipc";
import { useT } from "../../shared/i18n";
import { useActiveServer, useServers } from "../servers/store";
import { ErrorFolded, ErrorNotice } from "../shared/ErrorNotice";
import { basename } from "../shared/names";
import { VideoCard } from "./VideoCard";
import { VIDEO_EXTENSIONS, canStart, mergeListed, upsert } from "./rules";

export function VideoScreen() {
  const t = useT();
  const w = t.ui.video;
  const active = useActiveServer();
  const serversLoading = useServers((s) => s.loading);
  const reloadServers = useServers((s) => s.reload);
  const [videos, setVideos] = useState<VideoView[]>([]);
  const [refused, setRefused] = useState<VideoRefusal[]>([]);
  const [failures, setFailures] = useState<Record<string, AppError>>({});
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [params, setParams] = useSearchParams();
  const asked = useRef(false);

  useEffect(() => {
    void reloadServers();
  }, [reloadServers]);

  // Listening starts before the list is asked for, so a change in between is not lost; the
  // two are merged by `updated_at` whichever arrives first.
  useEffect(() => {
    let alive = true;
    const unlisten = onVideoUpdate((v) => {
      if (alive) setVideos((list) => upsert(list, v));
    });
    ipc
      .videoList()
      .then((listed) => {
        if (alive) setVideos((list) => mergeListed(list, listed));
      })
      .catch((e) => {
        if (alive) setError(toAppError(e));
      });
    return () => {
      alive = false;
      void unlisten.then((off) => off());
    };
  }, []);

  const add = useCallback(
    async (mediaId: string | null = null) => {
      if (!active) return;
      const chosen = await open({
        // A medium takes one film (T675); otherwise any number.
        multiple: mediaId === null,
        directory: false,
        filters: [{ name: w.pickFilter, extensions: VIDEO_EXTENSIONS }],
      });
      const paths = Array.isArray(chosen) ? chosen : typeof chosen === "string" ? [chosen] : [];
      if (paths.length === 0) return;
      setBusy(true);
      setError(null);
      try {
        const answer = await ipc.videoAdd(active.id, paths, mediaId);
        setVideos((list) => answer.added.reduce(upsert, list));
        setRefused(answer.refused);
      } catch (e) {
        setError(toAppError(e));
      } finally {
        setBusy(false);
      }
    },
    [active, w.pickFilter],
  );

  // The library's «Add video» lands here with `?add=1`, its «Build a set» with `?media=<id>`
  // (T675): the file dialog opens once, and the address is cleaned so going back to this
  // screen does not open it again.
  useEffect(() => {
    const media = params.get("media");
    if ((params.get("add") !== "1" && !media) || !active || asked.current) return;
    asked.current = true;
    setParams({}, { replace: true });
    void add(media);
  }, [params, active, add, setParams]);

  const fail = useCallback((id: string, e: AppError | null) => {
    setFailures((was) => {
      const next = { ...was };
      if (e) next[id] = e;
      else delete next[id];
      return next;
    });
  }, []);

  const startable = videos.filter((v) => v.state === "ready" && canStart(v));
  const startAll = async () => {
    setBusy(true);
    try {
      const answers = await ipc.videoStart(startable.map((v) => v.id));
      for (const a of answers) fail(a.id, a.error);
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  if (serversLoading) return <div className="panel">{t.ui.common.loading}</div>;

  return (
    <div className="panel">
      <div className="panel__head">
        <h1>{w.heading}</h1>
        <div className="panel__head-actions">
          <button
            type="button"
            className="button--primary"
            onClick={() => void add()}
            disabled={busy || !active}
          >
            {w.add}
          </button>
          <button
            type="button"
            onClick={() => void startAll()}
            disabled={busy || startable.length === 0}
          >
            {w.startAll}
          </button>
        </div>
      </div>

      {!active && (
        <p className="muted">
          {w.noServer} <Link to="/servers">{w.goToServers}</Link>
        </p>
      )}

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {refused.length > 0 && (
        <div className="notice notice--error video__refused" role="alert">
          <ul className="notice__body">
            {refused.map((r) => (
              <li key={r.path} data-testid="refused">
                <strong>{basename(r.path)}</strong>
                <ErrorFolded error={r.error} lineClassName="video__problem-line" />
              </li>
            ))}
          </ul>
          <button
            className="notice__close"
            onClick={() => setRefused([])}
            aria-label={w.refusedDrop}
          >
            ×
          </button>
        </div>
      )}

      {videos.length === 0 ? (
        <p className="muted">{w.empty}</p>
      ) : (
        <ul className="video-list">
          {videos.map((v) => (
            <VideoCard
              key={v.id}
              video={v}
              failure={failures[v.id] ?? null}
              onChanged={(next) => setVideos((list) => upsert(list, next))}
              onRemoved={(id) => setVideos((list) => list.filter((x) => x.id !== id))}
              onFailed={fail}
            />
          ))}
        </ul>
      )}
    </div>
  );
}
