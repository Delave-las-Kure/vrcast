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
  type DeployPreview,
  type DomainAnswer,
  type Ipv6Choice,
  type PlannedStep,
  type ServerState,
  type UpgradePlan,
  type ImportSuggestion,
  type LibraryChangedEvent,
  type LibraryView,
  type Links,
  type Settings,
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
  type LadderPlanRequest,
  type LadderBuildRequest,
  type QualityMeasureRequest,
  type LimitRequest,
  type GeoStatus,
  type UploadRequest,
  type Versions,
  type Viewer,
  type ViewersUpdateEvent,
  type LadderPreview,
  type LadderVerdict,
  type MeasurePreview,
  type Leftovers,
  type MeasurementView,
  type StoredMeasurement,
  type Rung,
  type SourceFacts,
  type SourceMeasured,
  type LimitPreview,
  type QualityLimit,
  type LadderServedVerdict,
  type Health,
  type Logs,
  type Stalls,
  type Peaks,
  type WhatWouldGo,
  type UpdateStanding,
  type Found,
  type WhatWent,
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
  /** Версии рядом (FR-128). Без сервера — только про приложение, и без соединения. */
  appVersions: (serverId?: string) =>
    call<Versions>("app_versions", { serverId: serverId ?? null }),

  // --- развёртывание ---
  /** Что это за сервер (FR-120). Смотреть можно на любой, который отвечает. */
  serverDetect: (serverId: string) => call<ServerState>("server_detect", { serverId }),
  /**
   * Ведёт ли домен сюда и сходится ли это с выбором про IPv6 (FR-137).
   *
   * Отдельная команда, чтобы человек мог проверить только что заведённую запись, не
   * начиная развёртывания: запись расходится по сети минутами, и ответ на «ещё нет» —
   * спросить снова, а не начать.
   */
  dnsCheck: (serverId: string, ipv6: Ipv6Choice) =>
    call<DomainAnswer>("dns_check", { serverId, ipv6 }),
  /** Что будет сделано, и ничего не делается (FR-122). */
  deployPlan: (serverId: string, ipv6: Ipv6Choice) =>
    call<DeployPreview>("deploy_plan", { serverId, ipv6 }),
  /** Развернуть. Без `confirmed` — отказ. Возвращает номер задачи (FR-080). */
  deployRun: (serverId: string, ipv6: Ipv6Choice, confirmed: boolean) =>
    call<string>("deploy_run", { serverId, ipv6, confirmed }),
  serverUpgradePlan: (serverId: string) => call<UpgradePlan>("server_upgrade_plan", { serverId }),
  serverUpgradeRun: (serverId: string, confirmed: boolean) =>
    call<string>("server_upgrade_run", { serverId, confirmed }),
  /** Вернуть то, что заменило последнее обновление (FR-133). */
  serverRollback: (serverId: string) => call<void>("server_rollback", { serverId }),

  // --- диагностика ---
  /** Как сервер себя чувствует (FR-070). */
  diagHealth: (serverId: string) => call<Health>("diag_health", { serverId }),
  /** Что раздача делала за промежуток (FR-071). */
  diagLogs: (serverId: string, minutes: number) => call<Logs>("diag_logs", { serverId, minutes }),
  /**
   * Почему встаёт картинка (FR-072).
   *
   * `file` — то, что нашёл `diagBitrate`, если его уже запускали. Без него вывод «виноват
   * файл» не выдаётся вовсе, и это правильно: файл в методе разбирается последним.
   */
  diagExplainStalls: (
    serverId: string,
    minutes: number,
    file?: { average_mbit: number; peak_10s_mbit: number },
  ) => call<Stalls>("diag_explain_stalls", { serverId, minutes, file: file ?? null }),
  /** Где пики у локального файла (FR-073). Сервер не трогается вовсе. */
  diagBitrate: (path: string) => call<Peaks>("diag_bitrate", { path }),

  // --- удаление (FR-114) ---
  /** Что уйдёт, если убрать всё. Ничего не меняет. */
  forgetPreview: () => call<WhatWouldGo>("forget_preview"),
  /** Убрать. Без `confirmed` — отказ, до того как что-либо тронуто. */
  forgetEverything: (confirmed: boolean) => call<WhatWent>("forget_everything", { confirmed }),

  // --- updating the application (FR-113) ---
  /** Version and packaging. Answers from this machine alone — nothing leaves it. */
  updateStanding: () => call<UpdateStanding>("update_standing"),
  /** Whether there is a newer version. Runs when a person asks, and at no other time. */
  updateCheck: () => call<Found>("update_check"),
  /** Fetch it and put it on. Without `confirmed` — refused before anything is fetched. */
  updateInstall: (confirmed: boolean) => call<void>("update_install", { confirmed }),

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
  serverAdd: (input: ServerInput, secret: string) => call<string>("server_add", { input, secret }),
  serverUpdate: (id: string, input: ServerInput, secret: string | null) =>
    call<void>("server_update", { id, input, secret }),
  serverRemove: (id: string) => call<void>("server_remove", { id }),
  serverSetActive: (id: string) => call<void>("server_set_active", { id }),
  serverTest: (id: string) => call<TestStep[]>("server_test", { id }),
  serverFingerprintConfirm: (id: string, fingerprint: string) =>
    call<void>("server_fingerprint_confirm", { id, fingerprint }),
  serverImportSuggestion: () => call<ImportSuggestion | null>("server_import_suggestion"),

  // --- library ---
  libraryList: (serverId: string, refresh = false) =>
    call<LibraryView>("library_list", { serverId, refresh }),
  mediaCreate: (serverId: string, title: string, slug: string | null) =>
    call<string>("media_create", { serverId, title, slug }),
  mediaRename: (serverId: string, mediaId: string, title: string | null, slug: string | null) =>
    call<void>("media_rename", { serverId, mediaId, title, slug }),
  mediaDelete: (serverId: string, mediaId: string, confirmed: boolean) =>
    call<string>("media_delete", { serverId, mediaId, confirmed }),
  fileMove: (serverId: string, path: string, toMediaId: string, confirmed: boolean) =>
    call<void>("file_move", { serverId, path, toMediaId, confirmed }),
  fileDelete: (serverId: string, path: string, confirmed: boolean) =>
    call<void>("file_delete", { serverId, path, confirmed }),
  linksFor: (serverId: string, path: string) => call<Links>("links_for", { serverId, path }),

  // --- viewers ---
  /**
   * Switch the watching on. The list then arrives by itself on `viewers:update`.
   *
   * Repeating it for the same server does nothing and is not an error: it is the ordinary
   * thing to do when the screen is opened again.
   */
  viewersWatchStart: (serverId: string) => call<void>("viewers_watch_start", { serverId }),
  /** Switch it off. Quiet when nothing was being watched. */
  viewersWatchStop: () => call<void>("viewers_watch_stop"),
  /** Those who watched earlier in this session, and have since stopped (FR-055). */
  viewersHistory: () => call<Viewer[]>("viewers_history"),

  // --- quality ladders ---
  /** What the source averages and where it peaks, before a ladder is worked out. */
  ladderMeasure: (path: string) => call<SourceMeasured>("ladder_measure", { path }),
  /**
   * The ladder for this film: the measured one when it has been measured, and the
   * formula's preview when it has not. The answer says which, every time.
   */
  ladderPlan: (request: LadderPlanRequest) => call<LadderPreview>("ladder_plan", { request }),
  /**
   * Check rungs a person has edited. Called on **every** edit (FR-044): a pure function
   * in the core, so it costs nothing and never waits on a file or a server.
   */
  ladderValidate: (rungs: Rung[], source: SourceFacts) =>
    call<LadderVerdict>("ladder_validate", { check: { rungs, source } }),

  // --- measuring quality ---
  qualityMeasurePreview: (request: QualityMeasureRequest) =>
    call<MeasurePreview>("quality_measure_preview", { request }),
  qualityMeasureStart: (request: QualityMeasureRequest) =>
    call<string>("quality_measure_start", { request }),
  /** What is still lying in a working folder — asked about the old one after the path is
   *  changed (T453). A reading, never an act: nothing is moved and nothing is deleted. */
  workDirLeftovers: (path: string) => call<Leftovers>("work_dir_leftovers", { path }),

  /** Stop a whole batch, waiting tasks included (T445). Returns how many were stopped, so
   *  the screen can say what happened rather than assert that something did. */
  tasksCancelBatch: (batchId: string) => call<number>("tasks_cancel_batch", { batchId }),

  /**
   * Every measurement already taken, so one can be offered to a film that has none (T427).
   *
   * All three of these were registered in the core, written into the contract, and called
   * from nowhere — found by the command comparison on 2026-08-28. The core could lend a
   * measurement from the first episode of a season to the second, and there was no way to
   * ask it to.
   */
  qualityMeasurements: () => call<StoredMeasurement[]>("quality_measurements", {}),

  /** Take another film's measurement for this one. Marked as borrowed on every rung. */
  qualityMeasureReuse: (fromKey: string, request: QualityMeasureRequest) =>
    call<MeasurementView>("quality_measure_reuse", { fromKey, request }),

  /** Throw a measurement away, so it can be taken again — the way out of a loan (T428). */
  qualityMeasureForget: (sourceKey: string, codec: string) =>
    call<void>("quality_measure_forget", { sourceKey, codec }),

  qualityMeasureResult: (sourceKey: string, codec: string) =>
    call<MeasurementView>("quality_measure_result", { sourceKey, codec }),

  /**
   * Build the set. Returns a task number at once: this is hours of work.
   *
   * Refuses before any task exists when the rungs were not measured — the core does that,
   * not the screen, so a way in that forgot to check cannot get past it either.
   */
  ladderBuild: (request: LadderBuildRequest) => call<string>("ladder_build", { request }),
  /** Ask the serving for every variant of a set (FR-047). */
  ladderVerify: (serverId: string, slug: string) =>
    call<LadderServedVerdict>("ladder_verify", { serverId, slug }),

  // --- quality limits ---
  /** What capping this viewer would do. Nothing is changed (FR-066). */
  limitPreview: (request: LimitRequest) => call<LimitPreview>("limit_preview", { request }),
  /**
   * Put the cap on. Refuses unless `confirmed`: what is being edited is the
   * configuration of the thing serving somebody's film at that moment.
   */
  limitSet: (request: LimitRequest, confirmed: boolean) =>
    call<void>("limit_set", { request, confirmed }),
  limitClear: (serverId: string, ip: string, slug: string) =>
    call<void>("limit_clear", { serverId, ip, slug }),
  // --- the tables of places ---
  /**
   * Whether the tables that turn an address into a place are there and current.
   *
   * Touches no server: the tables live on this machine.
   */
  geoStatus: () => call<GeoStatus>("geo_status"),
  /** Fetch this month's tables and put them to work without a restart. */
  geoUpdate: () => call<GeoStatus>("geo_update"),

  /** What is in force, read from the server rather than from a note here (FR-064). */
  limitsList: (serverId: string) => call<QualityLimit[]>("limits_list", { serverId }),

  // --- settings ---
  settingsGet: () => call<Settings>("settings_get"),
  settingsSet: (settings: Settings) => call<Settings>("settings_set", { settings }),

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
export function onTaskProgress(handler: (e: TaskProgressEvent) => void): Promise<UnlistenFn> {
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
export function onTaskNotify(handler: (e: TaskNotifyRequest) => void): Promise<UnlistenFn> {
  return tauriListen<TaskNotifyRequest>(EVENTS.taskNotify, (ev) => handler(ev.payload));
}

/**
 * The library has changed.
 *
 * The payload is an object rather than a string: the core tags the event with its
 * kind, as it does for task events, so that one cannot be mistaken for another.
 */
/**
 * The list of viewers, as it changes (FR-054).
 *
 * Listened to rather than asked for. The list moves every few seconds, and asking for it
 * that often would itself become the reason the interface stutters.
 */
export function onViewersUpdate(
  handler: (update: ViewersUpdateEvent) => void,
): Promise<UnlistenFn> {
  return tauriListen<ViewersUpdateEvent>(EVENTS.viewersUpdate, (ev) => handler(ev.payload));
}

/**
 * Развёртывание подвинулось на шаг (FR-123).
 *
 * Приходит **весь** список, а не один подвинувшийся шаг: экран, собирающий список из
 * потока одиночных, покажет другое, если один пропустит, — а он пропустит, потому что
 * человек открывает экран посередине.
 */
export function onDeployProgress(
  handler: (serverId: string, steps: PlannedStep[]) => void,
): Promise<UnlistenFn> {
  return tauriListen<{ server_id: string; steps: PlannedStep[] }>(EVENTS.deployProgress, (ev) =>
    handler(ev.payload.server_id, ev.payload.steps),
  );
}

export function onLibraryChanged(handler: (serverId: string) => void): Promise<UnlistenFn> {
  return tauriListen<LibraryChangedEvent>(EVENTS.libraryChanged, (ev) =>
    handler(ev.payload.server_id),
  );
}
