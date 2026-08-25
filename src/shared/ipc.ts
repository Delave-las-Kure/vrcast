/**
 * Talking to the core.
 *
 * The one place in the interface that knows how a call is made. Everything else calls
 * the typed functions from here and knows nothing of the shell or its events.
 *
 * Errors are brought to the shape of the contract here too. The core always answers
 * with `{ code, details, cause }`, but a call may never reach the core — and then
 * anything at all comes back, which must not be shown to a person as it is.
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  EVENTS,
  isAppError,
  type AppError,
  type ImportSuggestion,
  type LibraryChangedEvent,
  type LibraryView,
  type Links,
  type ServerInput,
  type ServerProfile,
  type Task,
  type TaskDoneEvent,
  type TaskOnClose,
  type TaskNotifyRequest,
  type TaskProgressEvent,
  type FfmpegInfo,
  type ConvertPreview,
  type ConvertStart,
  type SourceFile,
  type Validation,
  type TestStep,
  type UploadRequest,
  type Versions,
} from "./contract";

/**
 * An error that can be shown even when it did not come from the core.
 *
 * There is no wording here and there cannot be: it comes from the catalogue by code,
 * like every other error. Otherwise this one path would carry text living outside the
 * translation — and it would stay Russian under an English interface.
 */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return {
    code: "INTERNAL",
    cause: typeof e === "string" ? e : e instanceof Error ? e.message : undefined,
  };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    throw toAppError(e);
  }
}

// ---------- commands ----------

export const ipc = {
  appVersions: () => call<Versions>("app_versions"),

  tasksList: () => call<Task[]>("tasks_list"),
  taskGet: (id: string) => call<Task>("task_get", { id }),
  taskCancel: (id: string) => call<void>("task_cancel", { id }),
  taskPause: (id: string) => call<void>("task_pause", { id }),
  taskResume: (id: string) => call<void>("task_resume", { id }),
  /**
   * Reorder the waiting tasks (FR-083). `ordered` is the ids in the wanted order.
   * Returns how many were moved: some may have started while the list was on screen.
   */
  tasksReorder: (ordered: string[]) => call<number>("tasks_reorder", { ordered }),
  /** The ids of waiting tasks, in the order they will run. */
  tasksQueueOrder: () => call<string[]>("tasks_queue_order"),
  tasksOnClose: () => call<TaskOnClose[]>("tasks_on_close"),

  serverProbeFingerprint: (host: string, port: number) =>
    call<string>("server_probe_fingerprint", { host, port }),

  // --- servers ---
  serversList: () => call<ServerProfile[]>("servers_list"),
  serverAdd: (input: ServerInput, secret: string) =>
    call<string>("server_add", { input, secret }),
  serverUpdate: (id: string, input: ServerInput, secret: string | null) =>
    call<void>("server_update", { id, input, secret }),
  serverRemove: (id: string) => call<void>("server_remove", { id }),
  serverSetActive: (id: string) => call<void>("server_set_active", { id }),
  serverTest: (id: string) => call<TestStep[]>("server_test", { id }),
  serverFingerprintConfirm: (id: string, fingerprint: string) =>
    call<void>("server_fingerprint_confirm", { id, fingerprint }),
  serverImportSuggestion: () =>
    call<ImportSuggestion | null>("server_import_suggestion"),

  // --- library ---
  libraryList: (serverId: string, refresh = false) =>
    call<LibraryView>("library_list", { serverId, refresh }),
  mediaCreate: (serverId: string, title: string, slug: string | null) =>
    call<string>("media_create", { serverId, title, slug }),
  mediaRename: (
    serverId: string,
    mediaId: string,
    title: string | null,
    slug: string | null,
  ) => call<void>("media_rename", { serverId, mediaId, title, slug }),
  mediaDelete: (serverId: string, mediaId: string, confirmed: boolean) =>
    call<string>("media_delete", { serverId, mediaId, confirmed }),
  fileMove: (serverId: string, path: string, toMediaId: string, confirmed: boolean) =>
    call<void>("file_move", { serverId, path, toMediaId, confirmed }),
  fileDelete: (serverId: string, path: string, confirmed: boolean) =>
    call<void>("file_delete", { serverId, path, confirmed }),
  linksFor: (serverId: string, path: string) =>
    call<Links>("links_for", { serverId, path }),

  // --- upload ---
  /**
   * Start an upload. Returns the task id at once (FR-080).
   *
   * Every check happens before the start: if there is anything to warn about, the
   * command refuses and names the consequences. Repeating with `confirmed: true` is
   * agreeing to them. A shortage of room is not lifted that way: agreement does not
   * create space.
   */
  uploadStart: (request: UploadRequest) => call<string>("upload_start", { request }),
  uploadResume: (taskId: string) => call<void>("upload_resume", { taskId }),

  // --- preparing files ---
  /**
   * Check the bundled FFmpeg. Called at start-up and before preparation: learning
   * that FFmpeg does not work at the beginning means telling someone what to fix;
   * learning it halfway through a two-hour job means taking those two hours away.
   */
  ffmpegProbeSelf: () => call<FfmpegInfo>("ffmpeg_probe_self"),
  /** Examine a source file. A quick operation, not a task (FR-020). */
  sourceProbe: (path: string) => call<SourceFile>("source_probe", { path }),
  /** What preparation is going to do — before it starts. */
  convertPreview: (request: ConvertStart) => call<ConvertPreview>("convert_preview", { request }),
  /** Start preparation. Returns the task id at once (FR-080). */
  convertStart: (request: ConvertStart) => call<string>("convert_start", { request }),
  /** Check that a prepared file plays (FR-027). */
  convertValidate: (path: string) => call<Validation>("convert_validate", { path }),
};

// ---------- events ----------

/**
 * Subscribing to task progress.
 *
 * The interface listens rather than polls: showing the progress of a task that runs
 * for hours by polling would make it the cause of the very stuttering we avoid.
 */
export function onTaskProgress(
  handler: (e: TaskProgressEvent) => void,
): Promise<UnlistenFn> {
  return tauriListen<TaskProgressEvent>(EVENTS.taskProgress, (ev) => handler(ev.payload));
}

export function onTaskDone(handler: (e: TaskDoneEvent) => void): Promise<UnlistenFn> {
  return tauriListen<TaskDoneEvent>(EVENTS.taskDone, (ev) => handler(ev.payload));
}

/**
 * The core has decided a task is worth a system notification (FR-084).
 *
 * The decision is the core's — only it knows whether the window is out of sight and
 * how long the task ran. The wording is the interface's: a notification is read by
 * the same person as everything else, and in the same language.
 */
export function onTaskNotify(
  handler: (e: TaskNotifyRequest) => void,
): Promise<UnlistenFn> {
  return tauriListen<TaskNotifyRequest>(EVENTS.taskNotify, (ev) => handler(ev.payload));
}

/**
 * The library has changed.
 *
 * The payload is an object rather than a string: the core tags the event with its
 * kind, as it does for task events, so that one cannot be mistaken for another.
 */
export function onLibraryChanged(handler: (serverId: string) => void): Promise<UnlistenFn> {
  return tauriListen<LibraryChangedEvent>(EVENTS.libraryChanged, (ev) =>
    handler(ev.payload.server_id),
  );
}
