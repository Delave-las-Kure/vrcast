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

// ---------- серверы ----------

export type AuthKind = "key" | "password";
export type Ipv6Mode = "keep" | "disable";

/**
 * Профиль сервера в том виде, в каком его отдаёт ядро.
 *
 * Поля под сам секрет здесь нет и быть не может: наружу уходит только `secret_ref` —
 * ссылка на запись в хранилище операционной системы (FR-090, FR-091).
 */
export interface ServerProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  auth_kind: AuthKind;
  secret_ref: string;
  key_path: string | null;
  domain: string;
  video_dir: string;
  cdn_base: string | null;
  /** `null` = отпечаток ещё не подтверждён, подключаться нельзя (FR-092). */
  host_fingerprint: string | null;
  ipv6_mode: Ipv6Mode | null;
  is_active: boolean;
}

/** Поля, которые интерфейс отправляет при создании и изменении профиля. */
export interface ServerInput {
  name: string;
  host: string;
  port: number;
  user: string;
  auth_kind: AuthKind;
  key_path: string | null;
  domain: string;
  /** `null` = каталог раздачи по умолчанию. */
  video_dir: string | null;
  cdn_base: string | null;
  ipv6_mode: Ipv6Mode | null;
}

/** `skipped` — до шага не дошли: остановились раньше (FR-003). */
export type StepStatus = "ok" | "failed" | "skipped";

export interface TestStep {
  id: string;
  title: string;
  status: StepStatus;
  detail: string | null;
}

/** Предложение перенести настройки из `server.env` (T043). */
export interface ImportSuggestion {
  source: string;
  needs_passphrase: boolean;
  input: ServerInput;
}

// ---------- библиотека ----------

export interface FileView {
  path: string;
  size_bytes: number;
  duration_s: number | null;
  width: number | null;
  height: number | null;
  bitrate_bps: number | null;
  video_codec: string | null;
  audio_codec: string | null;
  /** Ложь = заголовок не в начале файла: зритель будет ждать скачивания хвоста. */
  faststart_ok: boolean | null;
  /** Ложь = файл удалён или переименован мимо приложения (FR-018). */
  exists_on_server: boolean;
  origin_url: string;
  cdn_url: string | null;
}

export interface MediaView {
  id: string;
  title: string;
  slug: string;
  files: FileView[];
  ladders: string[];
  total_bytes: number;
  created_at: string;
}

export interface DiskUsage {
  total_bytes: number;
  free_bytes: number;
  used_by_videos_bytes: number;
}

export interface LibraryView {
  server_id: string;
  media: MediaView[];
  /** Файлы, которые не удалось отнести ни к одному медиа (FR-015). */
  unrecognized: FileView[];
  disk: DiskUsage | null;
  /** Истина = показано последнее известное состояние, сервер сейчас недоступен. */
  stale: boolean;
}

/** Зрительские ссылки на файл (FR-016). */
export interface Links {
  origin: string;
  cdn: string | null;
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
  /** Место в очереди: меньше — раньше. Меняется перестановкой (FR-083). */
  queue_order: number;
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

/** Библиотека сервера изменилась — её нужно перечитать. */
export interface LibraryChangedEvent {
  event: "library_changed";
  server_id: string;
}

// ---------- заливка ----------

/** Заявка на заливку (FR-030…FR-039). */
export interface UploadRequest {
  server_id: string;
  /** Путь к готовому файлу на этом компьютере. */
  local_path: string;
  /** Под каким именем файл станет виден зрителям. */
  remote_name: string;
  /** К какому медиа отнести. `null` — файл попадёт в «не распознано». */
  media_id: string | null;
  /** Предел скорости в **байтах** в секунду. `null` — не ограничивать. */
  limit_bps: number | null;
  /** Согласие на последствия, названные в предыдущем отказе. */
  confirmed: boolean;
}
