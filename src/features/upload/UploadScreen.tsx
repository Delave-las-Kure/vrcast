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
 */

import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useSearchParams } from "react-router-dom";
import type { AppError, LibraryView, UploadRequest } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT, type Catalogue } from "../../shared/i18n";
import { fill, renderError } from "../../shared/i18n/render";
import { isReady, useActiveServer, useServers } from "../servers/store";
import { basename } from "../shared/names";
import { ErrorNotice } from "../shared/ErrorNotice";
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

/** What came of queuing a pack of files: how many made it, and what stopped the rest. */
interface BatchSummary {
  ok: number;
  total: number;
  failures: Array<{ name: string; message: string; hint: string }>;
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
   */
  const take = (paths: string[]) => {
    setLocalPaths(paths);
    if (paths.length === 1 && !remoteName) setRemoteName(basename(paths[0]));
    setPreflight(null);
    setStartedTask(null);
    setBatchSummary(null);
    setBatchPreflight(null);
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
    const { paths, resolvedMediaId, forceConfirmed } = run;
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
          setBatchPreflight({ error: err, run: { paths, index, resolvedMediaId, ok, failures, forceConfirmed } });
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
    setBatchSummary({ ok, total: paths.length, failures });
    setBusy(false);
  };

  const send = async (confirmed: boolean) => {
    if (!active || localPaths.length === 0) return;
    setBusy(true);
    setError(null);

    let resolvedMediaId: string | null = mediaId === "" ? null : mediaId;
    if (usingNewMedia) {
      try {
        resolvedMediaId = await ipc.mediaCreate(active.id, newMediaTitle.trim(), null);
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
    });
  };

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
                {localPaths.length === 1 && (
                  <span className="form__value">{localPaths[0]}</span>
                )}
              </div>
              {localPaths.length > 1 && (
                <ul className="upload__file-list">
                  {localPaths.map((path) => (
                    <li key={path}>{basename(path)}</li>
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
