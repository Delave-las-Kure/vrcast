/**
 * T014 — the types of the contract between the interface and the core.
 *
 * The single source of types for the interface. The contract is described in
 * `specs/001-vrcast-studio/contracts/ipc-commands.md`; this is its TypeScript side.
 *
 * IMPORTANT: the lists of error and detail codes are checked against the core by a
 * test (`src-tauri/tests/contract/contract_sync.rs`). Add a code in the core and add
 * it here too, or the build fails. That is deliberate: a silent divergence between
 * contract and implementation would be found by a user, not by a build.
 */

// ---------- errors ----------

/** An error code. The list is fixed by the contract. */
export type ErrorCode =
  // reaching the server
  | "SSH_AUTH_FAILED"
  | "SSH_UNREACHABLE"
  | "HOST_KEY_CHANGED"
  | "HOST_KEY_UNCONFIRMED"
  | "HOST_KEY_IS_CERTIFICATE"
  | "KEY_NEEDS_PASSPHRASE"
  | "KEY_UNREADABLE"
  | "VIDEO_DIR_DENIED"
  // domain
  | "DOMAIN_NOT_SERVING"
  | "DOMAIN_NOT_POINTED"
  | "DOMAIN_POINTS_ELSEWHERE"
  | "IPV6_MISMATCH"
  // server state and deployment
  | "SERVER_FOREIGN"
  | "SERVER_TOO_NEW"
  | "DEPLOY_STEP_FAILED"
  | "SWAP_FAILED"
  // library
  | "SLUG_TAKEN"
  | "MANIFEST_CONFLICT"
  | "FILE_MISSING_ON_SERVER"
  | "FILE_IN_USE"
  // preparing files
  | "FFMPEG_BROKEN"
  | "NO_AUDIO_TRACKS"
  | "DECODE_VALIDATION_FAILED"
  | "NO_HW_ENCODER"
  | "LOCAL_DISK_FULL"
  // transfer
  | "REMOTE_DISK_FULL"
  | "CHECKSUM_MISMATCH"
  | "VIEWERS_ACTIVE"
  | "NAME_EXISTS"
  // quality ladders
  | "RUNG_ABOVE_SOURCE"
  | "BUFSIZE_TOO_LARGE"
  | "LEVEL_EXCEEDED"
  | "LADDER_INCOMPLETE"
  | "NO_LADDER_FOR_MEDIA"
  | "VMAF_UNAVAILABLE"
  | "LADDER_NOT_MEASURED"
  | "MEASUREMENT_NOT_FOUND"
  | "MEASUREMENT_DIFFERENT_MATERIAL"
  // web server configuration
  | "CADDY_VALIDATE_FAILED"
  | "CADDY_RELOAD_FAILED"
  // tasks
  | "TASK_CANCELLED"
  | "TASK_NOT_FOUND"
  | "TASK_BAD_TRANSITION"
  | "TASK_NOT_PAUSABLE"
  // input and confirmation
  | "INVALID_INPUT"
  | "CONFIRMATION_REQUIRED"
  // everything else
  | "STORAGE_FAILED"
  | "INTERNAL";

/**
 * A specific thing the core can say, finer than the error code.
 *
 * The list is checked against the core by a contract test
 * (`src-tauri/tests/contract/contract_sync.rs`), and every entry must have a wording
 * in both languages — the catalogues are typed by this union, so a missing one is a
 * build failure rather than a blank line on someone's screen.
 */
export type DetailCode =
  // server profile fields
  | "PROFILE_ID_EMPTY"
  | "PROFILE_NAME_EMPTY"
  | "PROFILE_NAME_TOO_LONG"
  | "PROFILE_NAME_TAKEN"
  | "PROFILE_HOST_EMPTY"
  | "PROFILE_HOST_NOT_BARE"
  | "PROFILE_PORT_RANGE"
  | "PROFILE_USER_EMPTY"
  | "PROFILE_USER_HAS_SPACES"
  | "PROFILE_SECRET_REF_EMPTY"
  | "PROFILE_KEY_PATH_REQUIRED"
  | "PROFILE_KEY_PATH_UNUSED"
  | "PROFILE_NOT_FOUND"
  | "FINGERPRINT_EMPTY"

  // domain field
  | "DOMAIN_EMPTY"
  | "DOMAIN_HAS_SPACES"
  | "DOMAIN_HAS_PATH"
  | "DOMAIN_HAS_USER_OR_PORT"
  | "DOMAIN_BAD_DOTS"
  | "DOMAIN_NO_DOT"
  | "DOMAIN_BAD_CHARS"

  // video directory field
  | "VIDEO_DIR_EMPTY"
  | "VIDEO_DIR_NOT_ABSOLUTE"
  | "VIDEO_DIR_HAS_DOTDOT"
  | "VIDEO_DIR_HAS_NEWLINE"
  | "VIDEO_DIR_AT_ROOT"

  // CDN address field
  | "CDN_BASE_NO_SCHEME"
  | "CDN_BASE_HAS_SPACES"
  | "CDN_BASE_INCOMPLETE"

  // short name (slug)
  | "SLUG_EMPTY"
  | "SLUG_TOO_LONG"
  | "SLUG_BAD_CHAR"
  | "SLUG_RESERVED"
  | "SLUG_UNMAKEABLE"

  // library
  | "MEDIA_TITLE_EMPTY"
  | "MEDIA_NOTHING_TO_CHANGE"
  | "MEDIA_NOT_FOUND"
  | "MEDIA_IS_SERVICE_ENTRY"
  | "RENAME_FAILED"
  | "DELETE_FILES_FAILED"
  | "MANIFEST_MALFORMED"
  | "CONFIRM_DELETE"
  | "VIEWERS_ACTIVE_DELETE"

  // preparing files
  | "FFMPEG_SELF_BROKEN"
  | "FFMPEG_NO_X264"
  | "PROBE_NO_VIDEO"
  | "PROBE_UNREADABLE"
  | "CONVERT_NO_OUT_PATH"
  | "CONVERT_OUT_OVERWRITES_SOURCE"
  | "CONVERT_VALIDATE_NO_FFMPEG"
  | "CONVERT_NO_ENCODER"
  | "PLAN_NO_AUDIO_TRACKS"
  | "PLAN_NO_SUCH_TRACK"
  | "PLAN_HEIGHT_ZERO"
  | "PLAN_HEIGHT_ABOVE_SOURCE"
  | "PLAN_BITRATE_ZERO"
  | "PLAN_BITRATE_ABOVE_SOURCE"

  // how a long task can end badly
  | "CONVERT_VALIDATION_FAILED"
  | "UPLOAD_SHORT"
  | "UPLOAD_CHECKSUM_MISMATCH"
  | "UPLOAD_SOURCE_CHANGED"
  | "UPLOAD_TOO_MANY_BREAKS"
  | "UPLOAD_SOURCE_UNREADABLE"

  // stages of a long task, as shown beside its progress
  | "STAGE_CONVERTING"
  | "STAGE_VALIDATING"
  | "STAGE_CHECKSUM"
  | "STAGE_MEASURING_QUALITY"
  | "STAGE_DONE"

  // what closing the application would do to a task (FR-086)
  | "ON_CLOSE_RESUMES_FROM"
  | "ON_CLOSE_RESTARTS_LOSING"
  | "ON_CLOSE_NOT_STARTED_YET"
  | "ON_CLOSE_MUST_RUN_AGAIN"

  // steps of the connection check (FR-003)
  | "STEP_NET_BANNER"
  | "STEP_NET_TIMEOUT"
  | "STEP_NET_SILENT_CLOSED"
  | "STEP_NET_SILENT"
  | "STEP_NET_NOT_SSH"
  | "STEP_LOGIN_FINGERPRINT_UNCONFIRMED"
  | "STEP_LOGIN_OK"
  | "STEP_VIDEO_DIR_OK"
  | "STEP_VIDEO_DIR_MISSING_OR_DENIED"
  | "STEP_DOMAIN_OK_NO_FILES"
  | "STEP_DOMAIN_FILE_NOT_SERVED"
  | "STEP_DOMAIN_OK"
  | "STEP_DOMAIN_EMPTY_BODY"
  | "STEP_DOMAIN_TIMEOUT"
  | "STEP_DOMAIN_NO_CONNECTION"
  | "SYSTEM_ERROR"

  // why a stream cannot simply be carried across (FR-022)
  | "REASON_VIDEO_NOT_H264"
  | "REASON_VIDEO_PIX_FMT"
  | "REASON_TONEMAP"
  | "REASON_RESIZE"
  | "REASON_TARGET_BITRATE"
  | "REASON_AUDIO_NOT_AAC"
  | "REASON_AUDIO_CHANNELS"
  | "REASON_AUDIO_TOO_FAT"

  // what to say about the choice of encoder (FR-026)
  | "NOTICE_PROBE_UNCALIBRATED"
  | "NOTICE_PROBE_FAILED"
  | "NOTICE_MEASUREMENT_BORROWED"
  | "NOTICE_MEASUREMENT_PARTIAL"
  | "NOTICE_NO_HARDWARE_FOUND"
  | "NOTICE_SOFTWARE_AS_ASKED"
  | "NOTICE_HARDWARE_FAILED"

  // transfer
  | "UPLOAD_FILE_UNREADABLE"
  | "UPLOAD_NOT_A_FILE"
  | "UPLOAD_NAME_EMPTY"
  | "UPLOAD_ALREADY_RUNNING"
  | "UPLOAD_NAME_RESERVED"
  | "NOT_ENOUGH_SPACE"
  | "NAME_WILL_BE_REPLACED"
  | "CDN_KEEPS_OLD_COPY"
  | "VIEWERS_ACTIVE_UPLOAD";

/** One thing to say, with the values to put into it. */
export interface Detail {
  key: DetailCode;
  /** Substitutions by name. Numbers arrive raw and are formatted for the language. */
  params?: Record<string, string | number>;
}

/**
 * An error from the core.
 *
 * No prose: the core names the situation, the interface words it (FR-105, FR-106).
 * `details` is what to say, in order — empty means the code's own wording says it all.
 */
export interface AppError {
  code: ErrorCode;
  details?: Detail[];
  /** The particulars: which file, which step, which address. May be absent. */
  cause?: string;
}

/** Tell an error of the contract from any other surprise. */
export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as AppError).code === "string"
  );
}

// ---------- servers ----------

export type AuthKind = "key" | "password";
export type Ipv6Mode = "keep" | "disable";

/**
 * A server profile as the core hands it over.
 *
 * There is no field for the secret itself, and there cannot be: only `secret_ref`
 * leaves the core — a reference to an entry in the operating system's store
 * (FR-090, FR-091).
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
  /** `null` = the fingerprint is not confirmed yet; connecting is not allowed (FR-092). */
  host_fingerprint: string | null;
  ipv6_mode: Ipv6Mode | null;
  is_active: boolean;
}

/** The fields the interface sends when creating or changing a profile. */
export interface ServerInput {
  name: string;
  host: string;
  port: number;
  user: string;
  auth_kind: AuthKind;
  key_path: string | null;
  domain: string;
  /** `null` = the default serving directory. */
  video_dir: string | null;
  cdn_base: string | null;
  ipv6_mode: Ipv6Mode | null;
}

/** `skipped` — the step was never reached: it stopped earlier (FR-003). */
export type StepStatus = "ok" | "failed" | "skipped";

export interface TestStep {
  /** Stable step name: `network`, `login`, `video_dir`, `domain`. The title is looked
   *  up by it, so it no longer travels with every step. */
  id: string;
  status: StepStatus;
  /** What to say about the outcome. Absent for a step that was not attempted. */
  detail: Detail | null;
}

/** An offer to carry settings over from `server.env` (T043). */
export interface ImportSuggestion {
  source: string;
  needs_passphrase: boolean;
  input: ServerInput;
}

// ---------- library ----------

export interface FileView {
  path: string;
  size_bytes: number;
  duration_s: number | null;
  width: number | null;
  height: number | null;
  bitrate_bps: number | null;
  video_codec: string | null;
  audio_codec: string | null;
  /** False = the header is not at the start: a viewer waits for the tail to download. */
  faststart_ok: boolean | null;
  /** False = the file was deleted or renamed outside the application (FR-018). */
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
  /** Files that could not be assigned to any medium (FR-015). */
  unrecognized: FileView[];
  disk: DiskUsage | null;
  /** True = this is the last known state; the server is out of reach right now. */
  stale: boolean;
}

/** The viewer links for a file (FR-016). */
export interface Links {
  origin: string;
  cdn: string | null;
}

// ---------- tasks ----------

export type TaskKind =
  | "probe"
  | "convert"
  | "upload"
  | "measure_quality"
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
  /** From 0 to 1. */
  progress: number;
  /** Which stage it is at, as a code. Absent when it has not said. */
  stage: DetailCode | null;
  speed_bps: number | null;
  eta_s: number | null;
  resume_token: string | null;
  /** Why it failed. An object, so a task from last week still explains itself in
   *  whatever language is chosen today. */
  error: AppError | null;
  /** Place in the queue: lower runs sooner. Changed by reordering (FR-083). */
  queue_order: number;
  created_at: string;
  updated_at: string;
}

/** What becomes of a task if the application is closed (FR-086). */
export interface TaskOnClose {
  id: string;
  kind: string;
  progress: number;
  /** `resumes` — continues from where it got to; `restarts` — starts over. */
  outcome: "resumes" | "restarts";
  /** What exactly happens to this one. A general "tasks are running, close anyway?"
   *  gives nothing to decide on. */
  explanation: Detail;
}

export interface Versions {
  app: string;
  /** The server-side version of the active server. Arrives in Phase 7. */
  server: number | null;
  schema: number;
}

// ---------- events ----------

/** Event names. Fixed by the contract. */

/** Why a viewer is marked as having trouble (FR-053). */
export type ViewerProblem = "SlowLink" | "Retransmits" | "Stalls";

/**
 * Somebody watching right now.
 *
 * Every field that may be absent means **not determined**, and is shown as that. Nothing
 * here is ever filled in by guessing: a city invented from a neighbouring range looks
 * exactly like knowledge (FR-052).
 */
export interface Viewer {
  ip: string;
  country: string | null;
  city: string | null;
  asn_org: string | null;
  /** What they are watching. Null while no request of theirs has been recorded yet. */
  media_id: string | null;
  variant: string | null;
  /** What is arriving. Null until there is enough measurement to work it out from. */
  delivery_bps: number | null;
  /** What the variant they are getting needs. */
  required_bps: number | null;
  started_at: string;
  last_seen_at: string;
  problems: ViewerProblem[];
}

/**
 * The list, as it arrives — not as it is asked for.
 *
 * The core sends this every few seconds while watching is on. The interface does not poll:
 * polling something that changes this often is what SC-009 exists to prevent.
 */
export interface ViewersUpdateEvent {
  event: "viewers_update";
  server_id: string;
  active: Viewer[];
  /** How many are watching each medium — for the card in the library (FR-056). */
  per_media: Record<string, number>;
}

/** What the person may change. */
export interface Settings {
  viewer_activity_threshold_s: number;
  /**
   * Whether an outside service may be asked to place an address more exactly.
   *
   * Off unless deliberately turned on (FR-057): asking hands a viewer's address to somebody
   * else, for every viewer, every session.
   */
  geo_refine_outside: boolean;
  concurrent_heavy_tasks: number;
  mascot: boolean;
  animations: boolean;
  language: string | null;
  theme: string | null;
}

export const EVENTS = {
  taskProgress: "task:progress",
  taskDone: "task:done",
  taskNotify: "task:notify",
  libraryChanged: "library:changed",
  serverState: "server:state",
  viewersUpdate: "viewers:update",
} as const;

export interface TaskProgressEvent {
  event: "progress";
  id: string;
  state: TaskState;
  progress: number;
  stage: DetailCode | null;
  speed_bps: number | null;
  eta_s: number | null;
}

export interface TaskDoneEvent {
  event: "done";
  id: string;
  state: TaskState;
  error: AppError | null;
}

/**
 * The core has decided a system notification is warranted (FR-084).
 *
 * The decision is the core's — only it knows whether the window is out of sight and
 * how long the task ran. The wording is the interface's, like every other wording.
 */
export interface TaskNotifyRequest {
  id: string;
  kind: TaskKind;
  state: TaskState;
  error?: AppError;
}

/** A server's library has changed and needs reading again. */
export interface LibraryChangedEvent {
  event: "library_changed";
  server_id: string;
}

// ---------- upload ----------

/** A request to upload (FR-030 to FR-039). */
export interface UploadRequest {
  server_id: string;
  /** The path to the prepared file on this computer. */
  local_path: string;
  /** The name viewers will see the file under. */
  remote_name: string;
  /** Which medium to assign it to. `null` puts it in "not recognised". */
  media_id: string | null;
  /** The speed cap in **bytes** per second. `null` means no cap. */
  limit_bps: number | null;
  /** Agreement to the consequences named in the previous refusal. */
  confirmed: boolean;
}

// ---------- preparing files ----------

/** What could be learnt about the FFmpeg that ships with the application (FR-112). */
export interface FfmpegInfo {
  /** The version string as the program itself gives it. */
  version: string;
  /** The full path — needed when investigating trouble. */
  path: string;
  /** Whether the software H.264 encoder is there. Without it, preparation is impossible. */
  has_x264: boolean;
  has_libvmaf: boolean;
  /**
   * The hardware encoders this build knows how to call.
   *
   * "Knows how to call" is not "works here": presence in the build says nothing about
   * the graphics card. The real check is a trial run (FR-026).
   */
  hardware: string[];
}

/** An audio track of the source (FR-020, FR-021). */
export interface AudioTrack {
  /** The index among audio tracks, from zero. This is what ffmpeg understands. */
  index: number;
  codec: string;
  channels: number;
  bitrate_bps: number | null;
  /** The language. Often missing, which is ordinary rather than a fault. */
  language: string | null;
  title: string | null;
  is_default: boolean;
}

/** A source file that has been examined (data-model section 6). */
export interface SourceFile {
  path: string;
  size_bytes: number;
  duration_s: number;
  width: number;
  height: number;
  /** Frames per second, rounded up: 47.952 is 48-frame material. */
  fps: number;
  bitrate_bps: number;
  peak_bps: number | null;
  video_codec: string;
  pix_fmt: string;
  color_transfer: string | null;
  audio_tracks: AudioTrack[];
}

/** What to do with the video stream (FR-022). */
export type VideoAction =
  | { kind: "copy" }
  | { kind: "reencode"; reason: Detail; level: string }
  | {
      kind: "reencode_capped";
      reason: Detail;
      level: string;
      target_kbps: number;
      maxrate_kbps: number;
      bufsize_kbps: number;
    };

/** What to do with the audio. */
export type AudioAction =
  | { kind: "copy" }
  | { kind: "reencode"; reason: Detail; bitrate_kbps: number; resample_fix: boolean };

/** The plan for preparing a file. */
export interface ConvertPlan {
  video: VideoAction;
  audio: AudioAction;
  audio_track: number;
  gop: number;
  tonemap: boolean;
  requested_height: number | null;
  faststart: boolean;
}

/** What to encode with. */
export type Encoder = { kind: "hardware"; name: string } | { kind: "software" };

/** What preparation is going to do — shown before it starts. */
export interface ConvertPreview {
  plan: ConvertPlan;
  source: SourceFile;
  encoder: Encoder;
  /** What to say about the choice of encoder. Absent means there is nothing to say. */
  encoder_notice: Detail | null;
  /** True = nothing is re-encoded: minutes rather than hours, and no loss. */
  lossless: boolean;
}

/** A request to prepare a file. */
export interface ConvertStart {
  path: string;
  audio_track: number;
  target_kbps: number | null;
  height: number | null;
  out_path: string;
  /** False = the person asked for the processor themselves. */
  prefer_hardware: boolean;
}

/** The verdict of the playback check (FR-027). */
export interface Validation {
  /** Whether the file may be offered for upload. */
  ok: boolean;
  /** The decoder's complaints, in its own words. */
  problems: string[];
  /** The muxer's complaints, deliberately not held against the file. */
  ignored: string[];
}
