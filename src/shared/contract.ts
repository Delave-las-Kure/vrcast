/**
 * T014 — типы договора между интерфейсом и ядром.
 *
 * Единственный источник типов для интерфейса. Договор описан в
 * `specs/001-vrcast-studio/contracts/ipc-commands.md`; здесь его отражение на TypeScript.
 *
 * ВАЖНО: перечень кодов ошибок сверяется тестом ядра
 * (`src-tauri/tests/contract/contract_sync.rs`). Добавили код в ядре — добавьте и сюда,
 * иначе сборка упадёт. Это сделано намеренно: молчаливое расхождение договора и
 * реализации обнаружилось бы у пользователя, а не при сборке.
 */

// ---------- ошибки ----------

/** Код ошибки. Перечень закреплён договором. */
export type ErrorCode =
  // доступ к серверу
  | "SSH_AUTH_FAILED"
  | "SSH_UNREACHABLE"
  | "HOST_KEY_CHANGED"
  | "HOST_KEY_UNCONFIRMED"
  | "HOST_KEY_IS_CERTIFICATE"
  | "KEY_NEEDS_PASSPHRASE"
  | "KEY_UNREADABLE"
  | "VIDEO_DIR_DENIED"
  // домен
  | "DOMAIN_NOT_SERVING"
  | "DOMAIN_NOT_POINTED"
  | "DOMAIN_POINTS_ELSEWHERE"
  | "IPV6_MISMATCH"
  // состояние и развёртывание сервера
  | "SERVER_FOREIGN"
  | "SERVER_TOO_NEW"
  | "DEPLOY_STEP_FAILED"
  | "SWAP_FAILED"
  // библиотека
  | "SLUG_TAKEN"
  | "MANIFEST_CONFLICT"
  | "FILE_MISSING_ON_SERVER"
  | "FILE_IN_USE"
  // подготовка файлов
  | "FFMPEG_BROKEN"
  | "NO_AUDIO_TRACKS"
  | "DECODE_VALIDATION_FAILED"
  | "NO_HW_ENCODER"
  | "LOCAL_DISK_FULL"
  // передача
  | "REMOTE_DISK_FULL"
  | "CHECKSUM_MISMATCH"
  | "VIEWERS_ACTIVE"
  | "NAME_EXISTS"
  // наборы качеств
  | "RUNG_ABOVE_SOURCE"
  | "BUFSIZE_TOO_LARGE"
  | "LEVEL_EXCEEDED"
  | "LADDER_INCOMPLETE"
  | "NO_LADDER_FOR_MEDIA"
  // настройки веб-сервера
  | "CADDY_VALIDATE_FAILED"
  | "CADDY_RELOAD_FAILED"
  // задачи
  | "TASK_CANCELLED"
  | "TASK_NOT_FOUND"
  | "TASK_BAD_TRANSITION"
  | "TASK_NOT_PAUSABLE"
  // ввод и подтверждение
  | "INVALID_INPUT"
  | "CONFIRMATION_REQUIRED"
  // прочее
  | "STORAGE_FAILED"
  | "INTERNAL";

/**
 * Ошибка от ядра.
 *
 * `message` и `hint` уже готовы к показу на русском — сочинять свои формулировки
 * в интерфейсе не нужно и вредно: они разойдутся между экранами.
 */
export interface AppError {
  code: ErrorCode;
  message: string;
  hint: string;
  /** Уточнение случая: какой файл, какой шаг, какой адрес. Может отсутствовать. */
  cause?: string;
}

/** Отличить ошибку договора от любой другой неожиданности. */
export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as AppError).code === "string" &&
    typeof (e as AppError).message === "string"
  );
}

// ---------- задачи ----------

export type TaskKind =
  | "probe"
  | "convert"
  | "upload"
  | "build_ladder"
  | "deploy"
  | "upgrade_server"
  | "diagnose";

export type TaskState =
  | "queued"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export interface Task {
  id: string;
  kind: TaskKind;
  server_id: string | null;
  state: TaskState;
  /** От 0 до 1. */
  progress: number;
  stage: string | null;
  speed_bps: number | null;
  eta_s: number | null;
  resume_token: string | null;
  error: string | null;
  created_at: string;
  updated_at: string;
}

/** Что станет с задачей при закрытии приложения (FR-086). */
export interface TaskOnClose {
  id: string;
  kind: string;
  progress: number;
  /** `resumes` — продолжится с места; `restarts` — начнётся заново. */
  outcome: "resumes" | "restarts";
  /** Готовая строка для показа. Общего «идут задачи, закрыть?» недостаточно. */
  explanation: string;
}

export interface Versions {
  app: string;
  /** Версия серверной части активного сервера. Появится в Фазе 7. */
  server: number | null;
  schema: number;
}

// ---------- события ----------

/** Имена событий. Закреплены договором. */
export const EVENTS = {
  taskProgress: "task:progress",
  taskDone: "task:done",
  libraryChanged: "library:changed",
  serverState: "server:state",
  viewersUpdate: "viewers:update",
} as const;

export interface TaskProgressEvent {
  event: "progress";
  id: string;
  state: TaskState;
  progress: number;
  stage: string | null;
  speed_bps: number | null;
  eta_s: number | null;
}

export interface TaskDoneEvent {
  event: "done";
  id: string;
  state: TaskState;
  error: string | null;
}
