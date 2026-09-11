/**
 * T098 — the upload screen.
 *
 * What happens here: choose a prepared file, say what name viewers will see it under
 * and which medium it belongs to, cap the speed if wanted, and queue the task. After
 * that everything is visible in the task section: an upload runs for hours, and
 * nobody should have to keep this screen open for it.
 *
 * The file chooser is the system one (the `dialog` plugin). A web one will not do: a
 * file chosen there has no path on disk, and a path is exactly what an upload needs.
 *
 * The pre-flight checks are not repeated here. The core does them — it alone knows the
 * state of the server — and this screen only shows the answer and asks for agreement
 * where agreement is meaningful (see `PreflightWarnings`).
 *
 * T573 — several files can be chosen at once and bound to one medium (new or
 * existing), for a series uploaded episode by episode. One file behaves exactly as
 * before: the DOM and the existing tests must not tell the two apart. Several files
 * queue one `uploadStart` each, in order, unconfirmed to start — a failed file whose
 * refusal cannot be lifted by agreement (`REMOTE_DISK_FULL`) is simply reported and
 * skipped, the rest of the pack going on.
 *
 * T576 — a refusal that CAN be lifted by agreement (`VIEWERS_ACTIVE`, `NAME_EXISTS`,
 * `CONFIRMATION_REQUIRED`) pauses the run instead of being silently skipped: the same
 * `PreflightWarnings` the single-file flow shows comes up for that one file, and once
 * agreed to, the agreement holds for the rest of this run's remaining files — nobody
 * is asked the same question once per file in a series that all share one cause.
 *
 * T577 — a batch that creates its medium inline can still end up with that medium
 * holding no files at all: every file in the pack failed, or every liftable refusal
 * (T576) was declined rather than agreed to. The medium is real on the server and
 * empty, and the owner's call was to say so plainly rather than roll it back
 * silently — no deletion in this codebase ever happens without an explicit click
 * (see `ConfirmDeleteDialog`'s own doc-comment). `batchSummary` names it and offers
 * deleting it right there, through the same unconfirmed-then-confirmed dialog
 * `LibraryScreen` already uses for every other deletion.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useSearchParams } from "react-router-dom";
import type { AppError, LibraryView, UploadRequest } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT, type Catalogue } from "../../shared/i18n";
import { fill, renderError } from "../../shared/i18n/render";
import { isReady, useActiveServer, useServers } from "../servers/store";
import { basename } from "../shared/names";
import { ErrorNotice } from "../shared/ErrorNotice";
import { ConfirmDeleteDialog } from "../library/dialogs/MediaDialogs";
import { PreflightWarnings, canConfirm } from "./PreflightWarnings";

/**
 * Speed caps on offer. The values are bytes per second, as the core expects them.
 *
 * The value is fixed and the label is a catalogue key: the number means the same in
 * every language, the words around it do not.
 */
const LIMITS: Array<{ key: keyof Catalogue["ui"]["upload"]; value: number | null }> = [
  { key: "limitNone", value: null },
  { key: "limit10", value: 1_250_000 },
  { key: "limit25", value: 3_125_000 },
  { key: "limit50", value: 6_250_000 },
  { key: "limit100", value: 12_500_000 },
];

/**
 * The select's marker for "make a new medium here", rather than choosing one that
 * exists. Guaranteed not to collide with a real `media.id` — those come from the
 * core as server-side UUIDs/strings, never this literal.
 */
const NEW_MEDIA_MARKER = "__new__";

/** A medium created inline for this run, named so it can be offered for deletion. */
interface NewMedium {
  id: string;
  title: string;
}

/** What came of queuing a pack of files: how many made it, and what stopped the rest. */
interface BatchSummary {
  ok: number;
  total: number;
  failures: Array<{ name: string; message: string; hint: string }>;
  /** T577 — set only when this run created its medium inline AND not one file of the
   *  pack made it in: the medium exists on the server, real, and empty. `null` covers
   *  every other case — an existing medium was used, or at least one file landed. */
  orphanedMedia: NewMedium | null;
}

/**
 * T576 — where a paused batch run stands: which files are left, what has already been
 * settled, and whether agreement has already been given once this run (in which case it
 * holds for the rest of the pack, not just the file that first asked).
 */
interface BatchRun {
  paths: string[];
  index: number;
  resolvedMediaId: string | null;
  ok: number;
  failures: BatchSummary["failures"];
  forceConfirmed: boolean;
  /** T577 — non-null only when `resolvedMediaId` names a medium THIS run created,
   *  rather than one already on the server. Carried through pause/resume unchanged
   *  so the final summary can still tell the two apart once the run finishes. */
  newMedia: NewMedium | null;
}

export function UploadScreen() {
  const profiles = useServers((s) => s.profiles);
  const reloadServers = useServers((s) => s.reload);
  const active = useActiveServer();

  const [localPaths, setLocalPaths] = useState<string[]>([]);
  const [remoteName, setRemoteName] = useState("");
  const [mediaId, setMediaId] = useState<string>("");
  const [newMediaTitle, setNewMediaTitle] = useState("");
  const [limitBps, setLimitBps] = useState<number | null>(null);

  const [library, setLibrary] = useState<LibraryView | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  /** A refusal before starting: shown by `PreflightWarnings`, not by the error notice.
   *  Only ever raised for a single file — a pack of files does not go through this. */
  const [preflight, setPreflight] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [startedTask, setStartedTask] = useState<string | null>(null);
  const [batchSummary, setBatchSummary] = useState<BatchSummary | null>(null);
  /** T576 — a batch paused mid-pack on a liftable refusal (`VIEWERS_ACTIVE`,
   *  `NAME_EXISTS`, `CONFIRMATION_REQUIRED`), waiting on the same `PreflightWarnings`
   *  the single-file flow uses, but keeping the rest of the pack's state alive so the
   *  run can pick up where it stopped rather than starting over. */
  const [batchPreflight, setBatchPreflight] = useState<{ error: AppError; run: BatchRun } | null>(
    null,
  );
  /** T577 — the orphaned-medium delete flow, offered from `batchSummary`. `null` when
   *  nothing is being asked. Mirrors `LibraryScreen`'s own dialog-open state: the first
   *  `mediaDelete` call goes unconfirmed, and the core's refusal is what supplies the
   *  numbers `ConfirmDeleteDialog` shows (there is nothing to confirm blind). Failures
   *  from either call go through the same `error`/`busy` the rest of the screen already
   *  uses — a second pair of flags would say nothing `busy`/`error` cannot already say.
   */
  const [orphanDelete, setOrphanDelete] = useState<{
    media: NewMedium;
    consequences: string;
  } | null>(null);
  /**
   * T586 — guards `askDeleteOrphan` against a second click while its own unconfirmed
   * `mediaDelete` is still in flight. `busy` does not cover this: it is set by
   * `runBatch` and `confirmDeleteOrphan`, never by `askDeleteOrphan` itself, so the
   * delete button stayed clickable for the whole round trip — a second click sent a
   * second parallel unconfirmed `mediaDelete`, and if the two resolved out of order
   * (or the person confirmed the first while the second was still pending), the
   * confirm dialog could reopen pointing at a medium that was already gone. Same
   * shape as `busy`: set at the start of `askDeleteOrphan`, cleared in its `finally`,
   * regardless of whether the request turned out to be stale (see `uploadGenRef`
   * above) — a stale request is still this button's own request, and the button must
   * not stay disabled forever once it settles.
   */
  const [orphanDeleteAsking, setOrphanDeleteAsking] = useState(false);
  /**
   * T582 — guards against a stale `askDeleteOrphan`/`confirmDeleteOrphan` response
   * landing after the person has already moved on to a new file pick. `askDeleteOrphan`
   * sends its unconfirmed `mediaDelete` and then awaits the network; if `take()` or
   * `dropFile()` runs in the meantime, they bump this counter, and the awaited call's
   * `catch`/success branch compares its own captured value against the current one —
   * a mismatch means the request is answering a question nobody is asking anymore, so
   * its result (a confirm dialog, an error, a summary update) is simply dropped rather
   * than popping up over state that has already moved on.
   */
  const uploadGenRef = useRef(0);
  const t = useT();
  const { lang } = useLang();
  const u = t.ui.upload;

  useEffect(() => {
    void reloadServers();
  }, [reloadServers]);

  // The media list is needed so the file can be assigned straight away. A server out
  // of reach is no reason to break the screen: uploading is impossible anyway, and
  // that is said separately.
  useEffect(() => {
    if (!active) {
      setLibrary(null);
      return;
    }
    let live = true;
    void ipc
      .libraryList(active.id)
      .then((v) => {
        if (live) setLibrary(v);
      })
      .catch(() => {
        if (live) setLibrary(null);
      });
    return () => {
      live = false;
    };
  }, [active]);

  const media = useMemo(() => library?.media ?? [], [library]);

  const [params] = useSearchParams();

  /**
   * Take files, however they arrived — chosen here or handed over by the preparation
   * screen (which always hands over exactly one).
   *
   * One function for both, so the two cannot come to differ: filling the name in from
   * the file name is the sort of thing that gets done on one path and forgotten on the
   * other. With more than one file the served name stops meaning anything — each file
   * keeps its own, from its own name — so the field is left alone.
   *
   * T580 — accumulates rather than replaces, the same as `BatchScreen.pick()`: a
   * person picking a season two folders at a time must not have the first folder's
   * files thrown away the moment the dialogue opens a second time. `new Set` keeps a
   * file chosen twice from appearing twice in the list.
   */
  const take = (newPaths: string[]) => {
    uploadGenRef.current += 1;
    const merged = [...new Set([...localPaths, ...newPaths])];
    setLocalPaths(merged);
    if (merged.length === 1 && !remoteName) setRemoteName(basename(merged[0]));
    setPreflight(null);
    setStartedTask(null);
    setBatchSummary(null);
    setBatchPreflight(null);
    setOrphanDelete(null);
  };

  /**
   * T580 — drop one file from an already-chosen list, the same pattern as
   * `BatchScreen`'s own remove button. If this brings the list back down to exactly
   * one file, the served name is filled in from it — same as choosing a single file
   * from the start — rather than being left blank as if nothing had ever been chosen.
   */
  const dropFile = (path: string) => {
    uploadGenRef.current += 1;
    const next = localPaths.filter((p) => p !== path);
    setLocalPaths(next);
    if (next.length === 1 && !remoteName) setRemoteName(basename(next[0]));
    setPreflight(null);
    setStartedTask(null);
    setBatchSummary(null);
    setBatchPreflight(null);
    setOrphanDelete(null);
  };

  const pick = async () => {
    const chosen = await open({
      multiple: true,
      directory: false,
      title: u.pickTitle,
      filters: [{ name: u.pickFilter, extensions: ["mp4", "mkv", "mov", "webm", "m4v"] }],
    });
    if (Array.isArray(chosen)) take(chosen);
  };

  // What the preparation screen handed over, taken once. Not on every render: a person who
  // then chooses a different file must not have this one put back under them.
  const handed = params.get("file");
  useEffect(() => {
    if (handed && localPaths.length === 0) take([handed]);
    // `take` is rebuilt every render and `localPaths` is what this is guarding against;
    // both in the list would put the handed file back the moment it was replaced.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handed]);

  const usingNewMedia = mediaId === NEW_MEDIA_MARKER;

  /**
   * T576 — carry a batch forward from wherever it stands. Runs until every file has
   * been tried, or until a liftable refusal (`VIEWERS_ACTIVE`, `NAME_EXISTS`,
   * `CONFIRMATION_REQUIRED`) turns up and nobody has agreed to it yet this run — in
   * which case it stops and shows `PreflightWarnings` for that one file, the same
   * component the single-file flow already uses. Once agreement is given once, it is
   * remembered for the rest of this run (`forceConfirmed`): the question is not asked
   * again file after file. A non-liftable refusal (`REMOTE_DISK_FULL` and the like)
   * never pauses anything — it is recorded and the run goes straight on, as before.
   */
  const runBatch = async (run: BatchRun) => {
    if (!active) return;
    let { index, ok } = run;
    const { paths, resolvedMediaId, forceConfirmed, newMedia } = run;
    const failures = [...run.failures];
    setBusy(true);

    while (index < paths.length) {
      const path = paths[index];
      const name = basename(path);
      const request: UploadRequest = {
        server_id: active.id,
        local_path: path,
        remote_name: name,
        media_id: resolvedMediaId,
        limit_bps: limitBps,
        confirmed: forceConfirmed,
      };
      try {
        await ipc.uploadStart(request);
        ok += 1;
        index += 1;
      } catch (e) {
        const err = toAppError(e);
        if (!forceConfirmed && canConfirm(err)) {
          setBatchPreflight({
            error: err,
            run: { paths, index, resolvedMediaId, ok, failures, forceConfirmed, newMedia },
          });
          setBusy(false);
          return;
        }
        const { message, hint } = renderError(err, t, lang);
        failures.push({ name, message, hint });
        index += 1;
      }
    }

    setStartedTask(null);
    setPreflight(null);
    setBatchPreflight(null);
    // T577 — this run created its own medium AND not one file of the pack made it
    // in: the medium sits on the server, real and empty, with nothing pointing back
    // at it from this screen. Named here rather than left for the library to notice.
    const orphanedByThisRun = newMedia && ok === 0 ? newMedia : null;
    setBatchSummary({ ok, total: paths.length, failures, orphanedMedia: orphanedByThisRun });
    setBusy(false);
  };

  const send = async (confirmed: boolean) => {
    if (!active || localPaths.length === 0) return;
    setBusy(true);
    setError(null);

    let resolvedMediaId: string | null = mediaId === "" ? null : mediaId;
    // T577 — remembered so a batch that ends with nothing queued can name exactly
    // this medium in its summary. `null` for an existing medium: only one THIS run
    // made can be orphaned by it, never one that was already on the server.
    let newMedia: NewMedium | null = null;
    if (usingNewMedia) {
      try {
        resolvedMediaId = await ipc.mediaCreate(active.id, newMediaTitle.trim(), null);
        newMedia = { id: resolvedMediaId, title: newMediaTitle.trim() };
      } catch (e) {
        // A pack with no real medium behind it is not queued at all — not one file of
        // it, if the person explicitly asked for a new medium to hold the whole pack.
        setError(toAppError(e));
        setBusy(false);
        return;
      }
    }

    if (localPaths.length === 1) {
      const request: UploadRequest = {
        server_id: active.id,
        local_path: localPaths[0],
        remote_name: remoteName.trim(),
        media_id: resolvedMediaId,
        limit_bps: limitBps,
        confirmed,
      };

      try {
        const taskId = await ipc.uploadStart(request);
        setStartedTask(taskId);
        setBatchSummary(null);
        setPreflight(null);
      } catch (e) {
        const err = toAppError(e);
        // A refusal that can be argued with and one that cannot are shown differently
        // — but both here, beside the button, rather than somewhere else.
        if (canConfirm(err) || err.code === "REMOTE_DISK_FULL") setPreflight(err);
        else setError(err);
      } finally {
        setBusy(false);
      }
      return;
    }

    // Several files, one medium: each queues its own `uploadStart`, in order. A
    // liftable refusal (VIEWERS_ACTIVE, NAME_EXISTS, CONFIRMATION_REQUIRED) pauses the
    // run and asks — via `runBatch`/`batchPreflight` — rather than skipping straight to
    // failure; a non-liftable one (REMOTE_DISK_FULL) is recorded and the pack goes on.
    await runBatch({
      paths: localPaths,
      index: 0,
      resolvedMediaId,
      ok: 0,
      failures: [],
      forceConfirmed: false,
      newMedia,
    });
  };

  /**
   * T577 — ask what deleting the orphaned medium would cost, the same two-call shape
   * `LibraryScreen.askBeforeDelete` uses: an unconfirmed `mediaDelete` is refused with
   * `CONFIRMATION_REQUIRED`, carrying the numbers (here always zero files, but the
   * core is still the one saying so) that `ConfirmDeleteDialog` shows. Nothing here is
   * deleted silently — the button only ever starts this same confirm-then-act path.
   */
  const askDeleteOrphan = async (media: NewMedium) => {
    if (!active) return;
    const gen = uploadGenRef.current;
    setOrphanDeleteAsking(true);
    try {
      await ipc.mediaDelete(active.id, media.id, false);
      if (gen !== uploadGenRef.current) return;
      // The core agreed without confirmation. That should not happen — but if it has,
      // the summary must stop naming a medium that is no longer there.
      setBatchSummary((prev) => (prev ? { ...prev, orphanedMedia: null } : prev));
    } catch (e) {
      if (gen !== uploadGenRef.current) return;
      const err = toAppError(e);
      if (err.code === "CONFIRMATION_REQUIRED") {
        setOrphanDelete({ media, consequences: renderError(err, t, lang).message });
      } else {
        setError(err);
      }
    } finally {
      setOrphanDeleteAsking(false);
    }
  };

  /** T577 — carry out the deletion asked for above, once agreed to. */
  const confirmDeleteOrphan = async () => {
    if (!active || !orphanDelete) return;
    const gen = uploadGenRef.current;
    setBusy(true);
    try {
      await ipc.mediaDelete(active.id, orphanDelete.media.id, true);
      if (gen !== uploadGenRef.current) return;
      setOrphanDelete(null);
      setBatchSummary((prev) => (prev ? { ...prev, orphanedMedia: null } : prev));
    } catch (e) {
      if (gen !== uploadGenRef.current) return;
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  // A plain local rather than `batchSummary.orphanedMedia` used inline: TypeScript
  // does not carry a property's narrowing into a closure (the `onClick` below), but it
  // does carry a `const` local's — and this is read from three places in the JSX.
  const orphanedMedia = batchSummary?.orphanedMedia ?? null;

  const ready =
    active !== null &&
    isReady(active) &&
    localPaths.length > 0 &&
    (localPaths.length !== 1 || remoteName.trim() !== "") &&
    (!usingNewMedia || newMediaTitle.trim() !== "");

  if (profiles.length === 0) {
    return (
      <div className="panel">
        <h1>{u.heading}</h1>
        <p className="muted">{u.noServers}</p>
      </div>
    );
  }

  return (
    <div className="panel">
      <h1>{u.heading}</h1>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {active === null ? (
        <p className="muted">{u.noActive}</p>
      ) : !isReady(active) ? (
        <p className="muted">{fill(u.notReady, { name: active.name }, t, lang)}</p>
      ) : (
        <>
          <p className="muted">{fill(u.lead, { name: active.name }, t, lang)}</p>

          <div className="form">
            <div className="form__row">
              <label htmlFor="upload-file">{u.fieldFile}</label>
              <div className="form__inline">
                <button id="upload-file" onClick={() => void pick()} disabled={busy}>
                  {u.pickFile}
                </button>
                {localPaths.length === 1 && <span className="form__value">{localPaths[0]}</span>}
              </div>
              {localPaths.length > 1 && (
                <ul className="upload__file-list">
                  {localPaths.map((path) => (
                    <li key={path}>
                      {basename(path)}{" "}
                      <button
                        type="button"
                        className="button-link"
                        onClick={() => dropFile(path)}
                        disabled={busy}
                        aria-label={fill(u.dropOneFile, { name: basename(path) }, t, lang)}
                      >
                        {u.dropFile}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {localPaths.length <= 1 ? (
              <div className="form__row">
                <label htmlFor="upload-name">{u.fieldName}</label>
                <input
                  id="upload-name"
                  value={remoteName}
                  onChange={(e) => {
                    setRemoteName(e.target.value);
                    setPreflight(null);
                  }}
                  placeholder="film_22.mp4"
                />
                <p className="form__hint">{u.nameHint}</p>
              </div>
            ) : (
              <div className="form__row">
                <p className="form__hint">{u.multipleNamesHint}</p>
              </div>
            )}

            <div className="form__row">
              <label htmlFor="upload-media">{u.fieldMedia}</label>
              <select
                id="upload-media"
                value={mediaId}
                onChange={(e) => setMediaId(e.target.value)}
              >
                <option value="">{u.mediaNone}</option>
                {media.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.title}
                  </option>
                ))}
                <option value={NEW_MEDIA_MARKER}>{u.mediaNew}</option>
              </select>
              {usingNewMedia && (
                <div className="form__row">
                  <label htmlFor="upload-new-media-title">{u.newMediaLabel}</label>
                  <input
                    id="upload-new-media-title"
                    value={newMediaTitle}
                    onChange={(e) => setNewMediaTitle(e.target.value)}
                    placeholder={u.newMediaPlaceholder}
                  />
                </div>
              )}
            </div>

            <div className="form__row">
              <label htmlFor="upload-limit">{u.fieldLimit}</label>
              <select
                id="upload-limit"
                value={limitBps === null ? "" : String(limitBps)}
                onChange={(e) => setLimitBps(e.target.value === "" ? null : Number(e.target.value))}
              >
                {LIMITS.map((limit) => (
                  <option key={limit.key} value={limit.value === null ? "" : String(limit.value)}>
                    {u[limit.key] as string}
                  </option>
                ))}
              </select>
              <p className="form__hint">
                {u.limitHintLead}{" "}
                {limitBps === null
                  ? u.limitHintUnlimited
                  : fill(u.limitHintCapped, { bytes: limitBps }, t, lang)}
                .
              </p>
            </div>
          </div>

          {preflight && (
            <PreflightWarnings
              error={preflight}
              busy={busy}
              onConfirm={() => void send(true)}
              onCancel={() => setPreflight(null)}
            />
          )}

          {batchPreflight && (
            <PreflightWarnings
              error={batchPreflight.error}
              busy={busy}
              onConfirm={() => {
                const { run } = batchPreflight;
                setBatchPreflight(null);
                void runBatch({ ...run, forceConfirmed: true });
              }}
              onCancel={() => {
                const { error: err, run } = batchPreflight;
                const name = basename(run.paths[run.index]);
                const { message, hint } = renderError(err, t, lang);
                setBatchPreflight(null);
                void runBatch({
                  ...run,
                  index: run.index + 1,
                  failures: [...run.failures, { name, message, hint }],
                });
              }}
            />
          )}

          {startedTask && (
            <div className="notice notice--ok" role="status">
              <div className="notice__body">
                <strong className="notice__message">{u.started}</strong>
                <p className="notice__hint">{u.startedHint}</p>
              </div>
            </div>
          )}

          {batchSummary && (
            <div
              className={`notice ${batchSummary.failures.length > 0 ? "notice--warning" : "notice--ok"}`}
              role="status"
            >
              <div className="notice__body">
                <strong className="notice__message">
                  {batchSummary.failures.length === 0
                    ? fill(u.startedBatchAll, { n: batchSummary.ok }, t, lang)
                    : fill(
                        u.startedBatchPartial,
                        { ok: batchSummary.ok, total: batchSummary.total },
                        t,
                        lang,
                      )}
                </strong>
                <p className="notice__hint">{u.startedHint}</p>
                {orphanedMedia && (
                  <div className="notice__orphan">
                    <p className="notice__hint">
                      {fill(u.orphanedMediaWarning, { title: orphanedMedia.title }, t, lang)}
                    </p>
                    <div className="notice__actions">
                      <button
                        type="button"
                        className="button--danger"
                        onClick={() => void askDeleteOrphan(orphanedMedia)}
                        disabled={busy || orphanDeleteAsking}
                      >
                        {u.orphanedMediaDelete}
                      </button>
                    </div>
                  </div>
                )}
                {batchSummary.failures.length > 0 && (
                  <ul>
                    {batchSummary.failures.map((f) => (
                      <li key={f.name}>
                        {f.name}: {f.message}
                        {f.hint && ` ${f.hint}`}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            </div>
          )}

          {orphanDelete && (
            <ConfirmDeleteDialog
              what={orphanDelete.media.title}
              consequences={orphanDelete.consequences}
              busy={busy}
              onCancel={() => setOrphanDelete(null)}
              onConfirm={() => void confirmDeleteOrphan()}
            />
          )}

          <div className="form__actions">
            <button
              className="button--primary"
              disabled={!ready || busy}
              onClick={() => void send(false)}
            >
              {busy ? u.checking : u.start}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
