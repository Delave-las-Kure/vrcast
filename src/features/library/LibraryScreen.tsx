/**
 * T054 — the library: media and their files.
 *
 * The library is medium-centric: a person thinks about the work, and the files are
 * its quality variants. So the list is a list of media, and the files open inside.
 *
 * It shows from cache at once and the refresh follows (FR-080): waiting for the
 * server before showing a list that is already known is pointless — over a slow
 * connection it is seconds of blank screen.
 *
 * Deletion takes two calls, and that is not a wasted round. The first — without
 * confirmation — is refused by the core, and the refusal names the consequences: how
 * many files, how much room, whether anything is being served right now. Those are
 * what the dialog shows. Asking "are you sure?" having named nothing is a way of
 * getting a yes while telling someone nothing.
 */

import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { AppError, LadderSetView, LibraryView, MediaView } from "../../shared/contract";
import { ipc, onLibraryChanged, onViewersUpdate, toAppError } from "../../shared/ipc";
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { formatBitrate, formatBytes, formatDuration, formatResolution, usedFraction } from "../../shared/i18n/format";
import { fill, renderError } from "../../shared/i18n/render";
import { useActiveServer, useServers } from "../servers/store";
import { ErrorNotice } from "../shared/ErrorNotice";
import { CopyLink } from "./CopyLink";
import { FileRow } from "./FileRow";
import { StaleBanner } from "./StaleBanner";
import { UnrecognizedGroup } from "./UnrecognizedGroup";
import { ConfirmDeleteDialog, CreateMediaDialog, RenameMediaDialog } from "./dialogs/MediaDialogs";

/** What is open on top of the list right now. */
type Dialog =
  | { kind: "create" }
  | { kind: "rename"; media: MediaView }
  | { kind: "delete"; media: MediaView; consequences: string }
  | { kind: "delete-file"; path: string; consequences: string }
  | null;

export function LibraryScreen() {
  const active = useActiveServer();
  const reloadServers = useServers((s) => s.reload);
  const serversLoading = useServers((s) => s.loading);

  const [view, setView] = useState<LibraryView | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [dialogError, setDialogError] = useState<AppError | null>(null);
  const [renameFileInUse, setRenameFileInUse] = useState(false);
  const t = useT();
  const { lang } = useLang();
  const [watchers, setWatchers] = useState<Record<string, number>>({});

  // Only listened to. The library is not asked again for a number that changes this
  // often — that is the polling SC-009 exists to prevent. When nobody is watching, the
  // event never comes and the cards simply show no count.
  useEffect(() => {
    const unlisten = onViewersUpdate((update) => setWatchers(update.per_media));
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    void reloadServers();
  }, [reloadServers]);

  const load = useCallback(
    async (refresh: boolean) => {
      if (!active) {
        setView(null);
        setLoading(false);
        return;
      }
      try {
        setView(await ipc.libraryList(active.id, refresh));
        setError(null);
      } catch (e) {
        setError(toAppError(e));
      } finally {
        setLoading(false);
      }
    },
    [active],
  );

  // The first showing comes from the cache, immediately.
  useEffect(() => {
    setLoading(true);
    void load(false);
  }, [load]);

  // Refreshes from the core: both what arrives in the background and what we asked for.
  useEffect(() => {
    let cancelled = false;
    const unlisten: Array<() => void> = [];
    const keep = (fn: () => void) => {
      if (cancelled) fn();
      else unlisten.push(fn);
    };

    void onLibraryChanged(() => void load(false)).then(keep);

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [load]);

  /** Carry out a change and read the library again. */
  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setDialogError(null);
    try {
      await fn();
      setDialog(null);
      await load(true);
    } catch (e) {
      setDialogError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  /**
   * Rename a medium.
   *
   * Unlike a deletion, this is not a two-step "what would it cost" question — the
   * form is already open and this is an ordinary submit that either goes through or
   * is refused with `FILE_IN_USE` (someone is watching the file right now). That
   * refusal carries no numbers of its own (T545), so there is nothing to compose —
   * only a way to say "go on anyway" without losing what was typed, which is why
   * this lives beside `act` rather than inside it: `act` always closes the dialog on
   * success and always treats a caught error as final, and this path needs to stay
   * open and retry once with `confirmed: true`.
   */
  const doRename = async (
    media: MediaView,
    title: string | null,
    slug: string | null,
    confirmed?: boolean,
  ) => {
    if (!active) return;
    setBusy(true);
    setDialogError(null);
    try {
      await ipc.mediaRename(active.id, media.id, title, slug, confirmed ?? false);
      setRenameFileInUse(false);
      setDialog(null);
      await load(true);
    } catch (e) {
      const err = toAppError(e);
      setDialogError(err);
      setRenameFileInUse(err.code === "FILE_IN_USE");
    } finally {
      setBusy(false);
    }
  };

  /**
   * Ask the core what deleting would cost, and show it.
   *
   * A refusal with the code CONFIRMATION_REQUIRED is not an error but the expected
   * answer: it *is* the request for confirmation, carrying the numbers the interface
   * has no way of knowing itself.
   */
  const askBeforeDelete = async (media: MediaView) => {
    if (!active) return;
    try {
      await ipc.mediaDelete(active.id, media.id, false);
      // The core agreed without confirmation. That should not happen, but if it
      // has, the list has to be brought into line with what is really there.
      await load(true);
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "CONFIRMATION_REQUIRED") {
        // The refusal carries the numbers; the wording of them is ours.
        setDialog({
          kind: "delete",
          media,
          consequences: renderError(err, t, lang).message,
        });
      } else {
        setError(err);
      }
    }
  };

  const askBeforeDeleteFile = async (path: string) => {
    if (!active) return;
    try {
      await ipc.fileDelete(active.id, path, false);
      await load(true);
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "CONFIRMATION_REQUIRED") {
        setDialog({
          kind: "delete-file",
          path,
          consequences: renderError(err, t, lang).message,
        });
      } else {
        setError(err);
      }
    }
  };

  if (serversLoading || loading) {
    return <div className="panel">{t.ui.library.reading}</div>;
  }

  if (!active) {
    return (
      <div className="panel">
        <h1>{t.ui.library.heading}</h1>
        <p className="muted">{t.ui.library.noActiveServer}</p>
        <Link className="button-link" to="/servers">
          {t.ui.library.goToServers}
        </Link>
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel__head">
        <h1>{t.ui.library.heading}</h1>
        <div className="panel__head-actions">
          <button onClick={() => void load(true)} disabled={busy}>
            {t.ui.common.refresh}
          </button>
          <button onClick={() => setDialog({ kind: "create" })} disabled={busy}>
            {t.ui.library.newMedia}
          </button>
        </div>
      </div>

      <p className="muted library__server">
        {t.ui.library.serverLine} <strong>{active.name}</strong> · {active.domain}
      </p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      {view?.stale && <StaleBanner onRetry={() => void load(true)} />}
      {view?.disk && <DiskBar disk={view.disk} t={t} lang={lang} />}

      {dialog?.kind === "create" && (
        <CreateMediaDialog
          busy={busy}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onCreate={(title, slug) => void act(() => ipc.mediaCreate(active.id, title, slug))}
        />
      )}
      {dialog?.kind === "rename" && (
        <RenameMediaDialog
          media={dialog.media}
          busy={busy}
          error={dialogError}
          fileInUse={renameFileInUse}
          onCancel={() => {
            setDialog(null);
            setRenameFileInUse(false);
          }}
          onRename={(title, slug, confirmed) =>
            void doRename(dialog.media, title, slug, confirmed)
          }
        />
      )}
      {dialog?.kind === "delete" && (
        <ConfirmDeleteDialog
          what={dialog.media.title}
          consequences={dialog.consequences}
          busy={busy}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onConfirm={() => void act(() => ipc.mediaDelete(active.id, dialog.media.id, true))}
        />
      )}
      {dialog?.kind === "delete-file" && (
        <ConfirmDeleteDialog
          what={dialog.path}
          consequences={dialog.consequences}
          busy={busy}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onConfirm={() => void act(() => ipc.fileDelete(active.id, dialog.path, true))}
        />
      )}

      {view && view.media.length === 0 && view.unrecognized.length === 0 ? (
        <p className="muted">{t.ui.library.empty}</p>
      ) : (
        <div className="media-list">
          {view?.media.map((m) => (
            <MediaCard
              key={m.id}
              media={m}
              watching={watchers[m.id] ?? 0}
              t={t}
              lang={lang}
              disabled={busy || view.stale}
              onRename={() => {
                setDialog({ kind: "rename", media: m });
                setDialogError(null);
                setRenameFileInUse(false);
              }}
              onDelete={() => void askBeforeDelete(m)}
              onDeleteFile={(path) => void askBeforeDeleteFile(path)}
              onMoveFile={(path, mediaId) =>
                void act(() => ipc.fileMove(active.id, path, mediaId, true))
              }
              elsewhere={view.media
                .filter((other) => other.id !== m.id)
                .map((other) => ({ id: other.id, title: other.title }))}
            />
          ))}
          {view && (
            <UnrecognizedGroup
              files={view.unrecognized}
              media={view.media}
              serverId={active.id}
              disabled={busy || view.stale}
              onAssign={(path, mediaId) =>
                void act(() => ipc.fileMove(active.id, path, mediaId, true))
              }
              onDelete={(path) => void askBeforeDeleteFile(path)}
            />
          )}
        </div>
      )}
    </div>
  );
}

function MediaCard({
  media,
  disabled,
  onRename,
  onDelete,
  onDeleteFile,
  onMoveFile,
  elsewhere,
  watching,
  t,
  lang,
}: {
  media: MediaView;
  /** How many are watching it right now (FR-056). Zero shows nothing at all. */
  watching: number;
  disabled?: boolean;
  onRename: () => void;
  onDelete: () => void;
  onDeleteFile: (path: string) => void;
  onMoveFile: (path: string, toMediaId: string) => void;
  /** The other media this file could go to. This one is left out: moving a file to where it
   *  already is asks for work that changes nothing, and offering it invites the question of
   *  what it would do. */
  elsewhere: { id: string; title: string }[];
  t: Catalogue;
  lang: Lang;
}) {
  const [open, setOpen] = useState(false);
  const missing = media.files.filter((f) => !f.exists_on_server).length;

  return (
    <section className="media">
      <button className="media__head" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <span className="media__title">{media.title}</span>
        <span className="media__facts">
          {fill(
            t.ui.library.mediaFacts,
            { n: media.files.length, bytes: media.total_bytes },
            t,
            lang,
          )}
          {media.ladders.length > 0 && t.ui.library.hasLadder}
          {watching > 0 && (
            <em className="media__watching">
              {watching} {t.ui.viewers.watchingNow}
            </em>
          )}
          {missing > 0 && (
            <em className="media__missing">
              {fill(t.ui.library.missingOnServer, { n: missing }, t, lang)}
            </em>
          )}
        </span>
      </button>

      {open && (
        <>
          <p className="muted media__note">
            {t.ui.library.shortName} <code>{media.slug}</code>
          </p>

          <ul className="file-list">
            {media.files.map((f) => (
              <div key={f.path} className="media-card__file">
                <FileRow file={f} onDelete={disabled ? undefined : onDeleteFile} />
                {/* ⚠ **Moving a file to another medium** (T530, FR-013). The command has
                    existed all along and was reachable from one place only — assigning a file
                    the catalogue had never heard of. From one medium to another there was no
                    way at all, which is half of what the requirement asks for and the half a
                    person needs after choosing the wrong medium at upload.
                    The same control as the unrecognised group's, and deliberately so: it is
                    the same act, and two ways of doing one thing is how they come to behave
                    differently. */}
                {elsewhere.length > 0 && !disabled && (
                  <label className="media-card__move">
                    <span>{t.ui.library.moveTo}</span>
                    <select
                      value=""
                      onChange={(e) => {
                        if (e.target.value) onMoveFile(f.path, e.target.value);
                      }}
                    >
                      <option value="">{t.ui.library.assignChoose}</option>
                      {elsewhere.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.title}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
              </div>
            ))}
          </ul>

          {media.ladders.length > 0 && (
            <div className="ladder-sets" data-testid={`ladder-sets-${media.id}`}>
              <p className="muted media__note">{t.ui.library.laddersHeading}</p>
              <ul className="file-list">
                {media.ladders.map((set) => (
                  <LadderSetRow key={set.path} set={set} t={t} lang={lang} />
                ))}
              </ul>
            </div>
          )}

          <div className="media__actions">
            <button onClick={onRename} disabled={disabled}>
              {t.ui.library.renameMedia}
            </button>
            <button className="button--danger" onClick={onDelete} disabled={disabled}>
              {t.ui.library.deleteMedia}
            </button>
          </div>
        </>
      )}
    </section>
  );
}

/** T529 — one variant of a built quality set, shown the same way a `FileRow` is: a name,
 *  what is actually known about it, and a way to copy its link. Unlike a `FileView`, a
 *  `LadderSetView` carries no codec information — that column is simply left off rather
 *  than shown as a dash, which would claim the core looked and found nothing rather than
 *  never having asked. */
function LadderSetRow({ set, t, lang }: { set: LadderSetView; t: Catalogue; lang: Lang }) {
  const l = t.ui.library;
  return (
    <li className={`file ${set.exists_on_server ? "" : "file--missing"}`}>
      <div className="file__head">
        <span className="file__name">{set.path}</span>
        <span className="file__size">{formatBytes(set.size_bytes, lang)}</span>
      </div>

      {/* Honest absence rather than a made-up value: a set not built by this application,
          or one whose `.facts` could not be read, says nothing about its resolution,
          bitrate or duration instead of showing a zero or a dash that looks measured. */}
      <div className="file__meta">
        {set.width !== null && set.height !== null && (
          <span title={l.resolution}>{formatResolution(set.width, set.height)}</span>
        )}
        {set.duration_s !== null && (
          <span title={l.duration}>{formatDuration(set.duration_s)}</span>
        )}
        {set.bitrate_bps !== null && (
          <span title={l.bitrate}>{formatBitrate(set.bitrate_bps, lang)}</span>
        )}
      </div>

      {!set.exists_on_server && <p className="file__warning">{l.missingWarning}</p>}

      <div className="file__actions">
        {/* `CopyLink` asks for a `FileView`; a `LadderSetView` is missing the codec fields
            it never reads, so a minimal object carrying only what it actually uses is
            built here rather than widening `CopyLink`'s own type for a screen that does
            not need the rest. */}
        <CopyLink
          file={{
            path: set.path,
            size_bytes: set.size_bytes,
            duration_s: set.duration_s,
            width: set.width,
            height: set.height,
            bitrate_bps: set.bitrate_bps,
            video_codec: null,
            audio_codec: null,
            faststart_ok: null,
            exists_on_server: set.exists_on_server,
            origin_url: set.origin_url,
            cdn_url: set.cdn_url,
          }}
        />
      </div>
    </li>
  );
}

/** Room on the server's disk (FR-017). */
function DiskBar({
  disk,
  t,
  lang,
}: {
  disk: NonNullable<LibraryView["disk"]>;
  t: Catalogue;
  lang: Lang;
}) {
  const used = usedFraction(disk.total_bytes, disk.free_bytes);
  return (
    <div className="disk">
      <div className="disk__facts">
        <span>
          {t.ui.library.diskFree} <strong>{formatBytes(disk.free_bytes, lang)}</strong>{" "}
          {t.ui.library.diskOf} {formatBytes(disk.total_bytes, lang)}
        </span>
        <span className="muted">
          {fill(t.ui.library.diskVideos, { bytes: disk.used_by_videos_bytes }, t, lang)}
        </span>
      </div>
      <div
        className="progress"
        role="progressbar"
        aria-valuenow={Math.round(used * 100)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={t.ui.library.diskLabel}
      >
        <div
          className={`progress__fill ${used > 0.9 ? "progress__fill--alarm" : ""}`}
          style={{ width: `${used * 100}%` }}
        />
      </div>
    </div>
  );
}
