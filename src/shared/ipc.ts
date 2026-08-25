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
  type Task,
  type TaskDoneEvent,
  type TaskOnClose,
  type TaskProgressEvent,
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
  tasksOnClose: () => call<TaskOnClose[]>("tasks_on_close"),

  serverProbeFingerprint: (host: string, port: number) =>
    call<string>("server_probe_fingerprint", { host, port }),
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

export function onLibraryChanged(handler: (serverId: string) => void): Promise<UnlistenFn> {
  return tauriListen<string>(EVENTS.libraryChanged, (ev) => handler(ev.payload));
}
