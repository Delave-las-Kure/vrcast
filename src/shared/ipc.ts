/**
 * Обращение к ядру.
 *
 * Единственное место в интерфейсе, знающее, как устроен вызов. Всё остальное вызывает
 * отсюда типизированные функции и не догадывается ни про оболочку, ни про её события.
 *
 * Здесь же ошибка приводится к виду договора. Ядро всегда отвечает объектом
 * `{ code, message, hint }`, но до ядра вызов может и не дойти — тогда наружу прилетит
 * что угодно, и показывать это пользователю нельзя.
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
  type TaskProgressEvent,
  type FfmpegInfo,
  type SourceFile,
  type TestStep,
  type UploadRequest,
  type Versions,
} from "./contract";

/** Ошибка, которую можно показать, даже если она пришла не от ядра. */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return {
    code: "INTERNAL",
    message: "Внутренняя ошибка приложения",
    hint: "Сообщите об этой ошибке. Если она повторяется, помогут журналы из раздела диагностики.",
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

// ---------- команды ----------

export const ipc = {
  appVersions: () => call<Versions>("app_versions"),

  tasksList: () => call<Task[]>("tasks_list"),
  taskGet: (id: string) => call<Task>("task_get", { id }),
  taskCancel: (id: string) => call<void>("task_cancel", { id }),
  taskPause: (id: string) => call<void>("task_pause", { id }),
  taskResume: (id: string) => call<void>("task_resume", { id }),
  /**
   * Переставить ждущие задачи (FR-083). `ordered` — номера в желаемом порядке.
   * Возвращает, сколько переставлено: часть могла начаться, пока список был на экране.
   */
  tasksReorder: (ordered: string[]) => call<number>("tasks_reorder", { ordered }),
  /** Номера ждущих задач в том порядке, в каком они пойдут в работу. */
  tasksQueueOrder: () => call<string[]>("tasks_queue_order"),
  tasksOnClose: () => call<TaskOnClose[]>("tasks_on_close"),

  serverProbeFingerprint: (host: string, port: number) =>
    call<string>("server_probe_fingerprint", { host, port }),

  // --- серверы ---
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

  // --- библиотека ---
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

  // --- заливка ---
  /**
   * Начать заливку. Возвращает номер задачи немедленно (FR-080).
   *
   * Все проверки идут до старта: если есть о чём предупредить, команда откажется
   * и назовёт последствия. Повторить с `confirmed: true` — согласиться с ними.
   * Нехватка места этим не снимается: места от согласия не появится.
   */
  uploadStart: (request: UploadRequest) => call<string>("upload_start", { request }),
  uploadResume: (taskId: string) => call<void>("upload_resume", { taskId }),

  // --- подготовка файлов ---
  /**
   * Проверить вложенный FFmpeg. Зовётся при запуске и перед подготовкой:
   * узнать о неработающем FFmpeg в начале — значит сказать, что чинить;
   * узнать в середине двухчасовой подготовки — значит отнять эти два часа.
   */
  ffmpegProbeSelf: () => call<FfmpegInfo>("ffmpeg_probe_self"),
  /** Разобрать исходник. Быстрая операция, а не задача (FR-020). */
  sourceProbe: (path: string) => call<SourceFile>("source_probe", { path }),
};

// ---------- события ----------

/**
 * Подписка на продвижение задач.
 *
 * Интерфейс слушает, а не опрашивает: показывать продвижение многочасовой задачи опросом
 * значило бы самому стать причиной подтормаживания, которого мы избегаем.
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
 * Библиотека изменилась.
 *
 * Полезная нагрузка — объект, а не строка: ядро рассылает событие с меткой вида,
 * как и события задач, чтобы одно нельзя было принять за другое.
 */
export function onLibraryChanged(handler: (serverId: string) => void): Promise<UnlistenFn> {
  return tauriListen<LibraryChangedEvent>(EVENTS.libraryChanged, (ev) =>
    handler(ev.payload.server_id),
  );
}
