/**
 * The English catalogue.
 *
 * Same keys as the Russian one, checked by the compiler rather than by attention: the
 * type comes from `ru`, so a key added there and forgotten here fails the build.
 *
 * The wordings are translations of intent, not of grammar. Where Russian says «Проверьте,
 * что сервер включён», English says "Check that the server is switched on" — but where a
 * literal rendering would read as machine output, the sentence is rewritten to say the
 * same thing the way an English speaker would say it.
 */

import type { Catalogue } from "./catalogue";

export const en: Catalogue = {
  errors: {
    // --- reaching the server ---
    SSH_AUTH_FAILED: {
      message: "The server refused the sign-in details",
      hint: "Check the user name and the password or key.",
    },
    SSH_UNREACHABLE: {
      message: "Could not reach the server",
      hint: "Check that the server is on and the address and port are right.",
    },
    HOST_KEY_CHANGED: {
      message: "The server's fingerprint has changed",
      hint: "If the server was rebuilt, confirm the new fingerprint. If not, do not connect.",
    },
    HOST_KEY_UNCONFIRMED: {
      message: "The server's fingerprint has not been confirmed yet",
      hint: "Compare it with your hosting panel and confirm it.",
    },
    HOST_KEY_IS_CERTIFICATE: {
      message: "The server presented a certificate instead of a key",
      hint: "Turn off the host certificate on the server.",
    },
    KEY_NEEDS_PASSPHRASE: {
      message: "The key is protected by a passphrase",
      hint: "Enter the passphrase for this key.",
    },
    KEY_UNREADABLE: {
      message: "The key file could not be read",
      hint: "Choose the private key, not the .pub file.",
    },
    VIDEO_DIR_DENIED: {
      message: "No access to the video directory on the server",
      hint: "Check the path and the user's permissions on it.",
    },

    // --- domain ---
    DOMAIN_NOT_SERVING: {
      message: "The domain is not serving video",
      hint: "Open Diagnostics.",
    },
    DOMAIN_NOT_POINTED: {
      message: "The domain is not attached to the server",
      hint: "Add an A record pointing at the server's address and wait a few minutes.",
    },
    DOMAIN_POINTS_ELSEWHERE: {
      message: "The domain leads to a different server",
      hint: "Point the A record at this server's address.",
    },
    IPV6_MISMATCH: {
      message: "The IPv6 choice does not match the domain records",
      hint: "Add an AAAA record for the server's IPv6, or disable IPv6 when deploying.",
    },

    // --- server state and deployment ---
    SERVER_NEEDS_UPGRADE: {
      message: "The server side needs updating",
      hint: "Update it from the server's card. Files are kept.",
    },
    SERVER_FOREIGN: {
      message: "Something else is already serving from this server",
      hint: "Use a clean server, or remove the other setup by hand.",
    },
    SERVER_TOO_NEW: {
      message: "The server side is newer than this application understands",
      hint: "Update the application.",
    },
    DEPLOY_STEP_FAILED: {
      message: "A deployment step did not go through",
      hint: "Run the deployment again — finished steps will not repeat.",
    },
    SWAP_FAILED: {
      message: "The swap file could not be created",
      hint: "Free at least 1 GB on the server's disk.",
    },
    DEPLOY_ALREADY_RUNNING: {
      message: "A deployment or upgrade of this server is already running",
      hint: "Wait for it to finish.",
    },
    ROLLBACK_NO_COPY: {
      message: "Nothing to put back: the server has no copy of its settings",
      hint: "A copy is made by each deployment and upgrade. Nothing on the server was changed.",
    },

    // --- library ---
    SLUG_TAKEN: {
      message: "That name is already taken",
      hint: "Choose another name.",
    },
    MANIFEST_CONFLICT: {
      message: "The catalogue was changed by another application",
      hint: "Refresh the list and try again.",
    },
    FILE_MISSING_ON_SERVER: {
      message: "The file is no longer on the server",
      hint: "Refresh the library.",
    },
    FILE_IN_USE: {
      message: "Someone is watching this file right now",
      hint: "Deleting or renaming will cut the viewing short. Wait, or confirm.",
    },
    MEDIA_BUSY: {
      message: "This medium is being processed by the server right now",
      hint: "Wait for the build or upload to finish, then try again.",
    },
    FILE_BUSY: {
      message: "This file is being processed by the server right now",
      hint: "Wait for the build or upload to finish, then try again.",
    },

    // --- preparing files ---
    FFMPEG_BROKEN: {
      message: "The video tool will not start",
      hint: "Reinstall the application.",
    },
    NO_AUDIO_TRACKS: {
      message: "The file has no audio track at all",
      hint: "Choose another file.",
    },
    DECODE_VALIDATION_FAILED: {
      message: "The finished file failed the playback check",
      hint: "Prepare the file again from the source.",
    },
    NO_HW_ENCODER: {
      message: "Hardware acceleration is not available",
      hint: "The processor encodes instead — slower, same quality.",
    },
    LOCAL_DISK_FULL: {
      message: "Not enough room on this computer's disk",
      hint: "Free up space on this computer and try again.",
    },

    // --- transfer ---
    REMOTE_DISK_FULL: {
      message: "Not enough room on the server's disk",
      hint: "Delete media you no longer need in the library.",
    },
    CHECKSUM_MISMATCH: {
      message: "The transferred file differs from the source",
      hint: "The file was not published. Start the upload again.",
    },
    VIEWERS_ACTIVE: {
      message: "Someone is watching right now",
      hint: "Viewers' playback may stall. Better to wait until they finish.",
    },
    NAME_EXISTS: {
      message: "A file with that name is already being served",
      hint: "Choose another name or confirm the replacement.",
    },

    // --- quality ladders ---
    RUNG_ABOVE_SOURCE: {
      message: "The quality rung is higher than the source itself",
      hint: "Lower the rung.",
    },
    BUFSIZE_TOO_LARGE: {
      message: "The buffer is too large for the chosen peak limit",
      hint: "Make the buffer about equal to the peak limit.",
    },
    LEVEL_EXCEEDED: {
      message: "The stream does not fit the chosen compatibility level",
      hint: "Lower the bitrate, the frame rate or the resolution.",
    },
    LADDER_INCOMPLETE: {
      message: "The quality ladder was not built in full",
      hint: "Run the build again — finished variants are kept.",
    },
    VMAF_UNAVAILABLE: {
      message: "This build of FFmpeg cannot measure quality",
      hint: "Reinstall the application.",
    },
    LADDER_OBJECTION: {
      message: "Stopped: there are objections to the ladder that came out",
      hint: "Open this video's rungs, fix or accept them, and build.",
    },
    LADDER_NOT_MEASURED: {
      message: "The quality of this material has not been measured yet",
      hint: "Run the measurement, or borrow the first episode's.",
    },
    MEASUREMENT_NOT_FOUND: {
      message: "There is no such measurement",
      hint: "Run the measurement again.",
    },
    MEASUREMENT_DIFFERENT_MATERIAL: {
      message: "That measurement was taken on different material",
      hint: "Measure this file on its own.",
    },
    LADDER_CHECK_PENDING: {
      message: "The borrowed measurement is still being checked",
      hint: "Wait about a minute and try again.",
    },
    MEASUREMENT_NOT_THIS_MATERIAL: {
      message: "The measurement did not fit this film",
      hint: "Measure this file on its own.",
    },
    NO_LADDER_FOR_MEDIA: {
      message: "This medium has no quality ladder",
      hint: "Build a quality set first.",
    },

    // --- web server configuration ---
    CADDY_VALIDATE_FAILED: {
      message: "The new server configuration turned out to be invalid",
      hint: "Serving continues as before. Please report this error.",
    },
    CADDY_RELOAD_FAILED: {
      message: "The server did not accept the new configuration",
      hint: "The previous settings were restored. Check Diagnostics.",
    },
    LIMITS_CONFLICT: {
      message: "Someone else changed the quality limits at the same moment",
      hint: "Reload the list and try again.",
    },
    LIMITS_ROLLBACK_FAILED: {
      message:
        "The change of limits did not go through, and putting the previous limits back was not confirmed",
      hint: "Reload the list: it shows the rules file, which may differ from what is serving.",
    },

    // --- tasks ---
    TASK_CANCELLED: {
      message: "The task was cancelled",
      hint: "Nothing to do.",
    },
    TASK_NOT_FOUND: {
      message: "Task not found",
      hint: "Refresh the task list.",
    },
    TASK_BAD_TRANSITION: {
      message: "The task is in a state this cannot be done from",
      hint: "Refresh the task list.",
    },
    TASK_NOT_PAUSABLE: {
      message: "A task of this kind cannot be paused",
      hint: "Cancel it and run it again.",
    },

    // --- "Remove everything" (FR-114, T643) ---
    FORGET_TASKS_RUNNING: {
      message: "Stop the tasks first",
      hint: "Let the tasks finish or cancel them. Nothing was removed.",
    },
    FORGET_IN_PROGRESS: {
      message: "The application's data is being removed right now",
      hint: "Until it ends, no new task starts. Wait and try again.",
    },
    FORGET_PREVIEW_STALE: {
      message: "The list of what would go has changed",
      hint: "Nothing was removed. Check the new list and confirm again.",
    },

    // --- input and confirmation ---
    INVALID_INPUT: {
      message: "The details entered will not do",
      hint: "Correct the field and try again.",
    },
    CONFIRMATION_REQUIRED: {
      message: "Confirmation needed",
      hint: "Confirm it. It cannot be undone.",
    },

    // --- updating the application itself ---
    UPDATE_CHECK_FAILED: {
      message: "Could not check for updates",
      hint: "Check the connection and try again.",
    },
    UPDATE_INSTALL_FAILED: {
      message: "Could not install the update",
      hint: "Download the installer from the releases page.",
    },

    // --- everything else ---
    STORAGE_FAILED: {
      message: "Could not reach local storage",
      hint: "Check disk space and access to the data folder.",
    },
    INTERNAL: {
      message: "An internal error in the application",
      hint: "Please report it, with the logs from Diagnostics.",
    },
    // --- videos in work (T672) ---
    VIDEO_NOT_FOUND: {
      message: "The video is not on the list",
      hint: "It was removed from the list. Refresh the screen.",
    },
    VIDEO_NOT_NOW: {
      message: "This cannot be done right now",
      hint: "The video is in another state. Refresh the screen.",
    },
    MEDIA_HAS_SET: {
      message: "A quality set under this name already exists",
      hint: "“Replace” removes a set nobody owns; a medium's own set is deleted in the library.",
    },
    MEDIA_SET_IN_WORK: {
      message: "A set for this medium is already on its way",
      hint: "Look for it on the Video screen.",
    },
    VIDEO_MEDIUM_GONE: {
      message: "The set's medium was deleted",
      hint: "“Retry” makes it again.",
    },
  },

  details: {
    // --- server profile fields ---
    PROFILE_ID_EMPTY: "The profile's internal number is empty.",
    PROFILE_NAME_EMPTY: "The profile needs a name — it is how you will tell your servers apart.",
    PROFILE_NAME_TOO_LONG: "The name is longer than {max} characters — shorten it.",
    PROFILE_NAME_TAKEN: "A profile named “{name}” already exists — choose another.",
    PROFILE_HOST_EMPTY: "Enter the server's address — an IP address or a name.",
    PROFILE_HOST_NOT_BARE:
      "The server address must not contain spaces or slashes — the address alone, not a link.",
    PROFILE_PORT_RANGE: "The port must be between 1 and 65535. The usual SSH port is 22.",
    PROFILE_USER_EMPTY: "Enter the user the application signs in as.",
    PROFILE_USER_HAS_SPACES: "The user name must not contain spaces.",
    PROFILE_SECRET_REF_EMPTY: "No reference to a secret in the system store was set.",
    PROFILE_KEY_PATH_REQUIRED: "Signing in by key needs the path to the private key file.",
    PROFILE_KEY_PATH_UNUSED: "Signing in by password does not use a key path — remove it.",
    PROFILE_AUTH_NEEDS_SECRET:
      "The sign-in method is changing — enter the new password or the key's passphrase.",
    PROFILE_NOT_FOUND: "There is no such server — its profile may have been deleted.",
    PROFILE_MAY_BE_CHANGED:
      "The profile may have been left changed. Open it, check the sign-in method and save.",
    FINGERPRINT_EMPTY: "The fingerprint is empty — there is nothing to confirm.",

    // --- what the single door says when it stays shut (T519(1)) ---
    SERVER_FOREIGN_WEB_SERVER_RUNNING:
      "Something called {name} is already running on this server — it is not our own serving.",
    SERVER_FOREIGN_CONFIG_WITHOUT_STATE:
      "A web-server configuration without our state file — set up by someone else.",
    SERVER_FOREIGN_STATE_UNREADABLE:
      "Our own state file is on this server but could not be read ({problem}).",
    SERVER_FOREIGN_UNKNOWN: "The server was judged foreign, but no reason was given.",
    SERVER_NOT_DEPLOYED: "Nothing is deployed on this server yet.",
    SERVER_ALREADY_DEPLOYED:
      "The server is already deployed and at a version this application is happy with.",

    // --- domain field ---
    DOMAIN_EMPTY: "Enter the domain you serve from.",
    DOMAIN_HAS_SPACES: "The domain must not contain spaces.",
    DOMAIN_HAS_PATH: "Enter the domain only, without a path: stream.example.com, say.",
    DOMAIN_HAS_USER_OR_PORT: "Enter the domain only — no user and no port.",
    DOMAIN_BAD_DOTS:
      "The domain is written wrongly: dots cannot sit at either end or follow one another.",
    DOMAIN_NO_DOT: "The domain must contain a dot: stream.example.com, say.",
    DOMAIN_BAD_CHARS: "A domain may only contain letters, digits, hyphens and dots.",

    // --- video directory field ---
    VIDEO_DIR_EMPTY: "Enter the video directory on the server.",
    VIDEO_DIR_NOT_ABSOLUTE: "The path must start from the root, with a slash.",
    VIDEO_DIR_HAS_DOTDOT: "The path must not contain “..” — give the directory in full.",
    VIDEO_DIR_HAS_NEWLINE: "The path must not contain a line break.",
    VIDEO_DIR_AT_ROOT: "The directory cannot be at the root of the file system.",

    // --- CDN address field ---
    CDN_BASE_NO_SCHEME: "The CDN address must begin with https:// or http://.",
    CDN_BASE_HAS_SPACES: "The CDN address must not contain spaces.",
    CDN_BASE_INCOMPLETE: "The CDN address is incomplete.",

    // --- short name (slug) ---
    SLUG_EMPTY: "The short name cannot be empty.",
    SLUG_TOO_LONG:
      "A short name of {len} characters will not fit into a file name — shorten it to {max}.",
    SLUG_BAD_CHAR:
      "The character “{char}” is not allowed in a short name: use Latin letters, digits, hyphens and underscores.",
    SLUG_RESERVED: "That short name is reserved for internal use — choose another.",
    SLUG_UNMAKEABLE:
      "No short name can be made from this title — set one yourself: Latin letters, digits, hyphens and underscores.",

    // --- library ---
    MEDIA_TITLE_EMPTY: "The title cannot be empty — it is how you will find the medium.",
    MEDIA_NOTHING_TO_CHANGE: "Nothing to change: neither a title nor a short name was given.",
    MEDIA_NOT_FOUND: "There is no such medium in the library — refresh the list.",
    MEDIA_IS_SERVICE_ENTRY: "This is an internal serving entry; it cannot be deleted here.",
    RENAME_FAILED: "Could not rename “{old}” to “{new}”.",
    DELETE_FILES_FAILED: "Could not delete the files on the server.",
    MANIFEST_MALFORMED: "The library catalogue on the server is corrupt and cannot be read.",
    CONFIRM_DELETE:
      "{files} {files|plural:file} ({bytes|bytes}) will be deleted. This cannot be undone.",
    VIEWERS_ACTIVE_DELETE: "Open connections: {connections}. Deleting may cut playback.",
    CONFIRM_DELETE_SET_FILES: "Among them, the set's rung files: {names}.",
    MEDIA_BUSY_BUILDING:
      "The quality set “{slug}” is being built on the server right now — deletion will wait until the build finishes.",
    MEDIA_BUSY_UPLOADING:
      "The file “{name}” is being uploaded to the server right now — deletion will wait until the upload finishes.",
    MEDIA_BUSY_VIDEO: "A set is being built into this medium — see Video.",

    // --- preparing files ---
    FFMPEG_SELF_BROKEN: "The bundled FFmpeg does not work. Reinstall the application.",
    FFMPEG_NO_X264:
      "The bundled FFmpeg has no software H.264 — without a graphics card there is nothing to encode with.",
    PROBE_NO_VIDEO: "There is no video in this file — perhaps the wrong file was chosen.",
    PROBE_UNREADABLE: "The file could not be parsed: it is damaged, or it is not video.",
    CONVERT_NO_OUT_PATH: "Where to put the prepared file was not specified.",
    CONVERT_OUT_OVERWRITES_SOURCE:
      "The prepared file cannot be written over the source — the source would be lost for good.",
    CONVERT_OUT_EXISTS:
      "A finished file is already there: {out_path}. Replace it? The old one stays in place until the new one is ready and checked.",
    CONVERT_OUT_BUSY:
      "Another preparation is already writing this file: {out_path}. Wait for it to end, or choose another place.",
    CONVERT_REPLACE_FAILED:
      "Could not replace {out_path} (open elsewhere?). The new file: {kept_at}",
    VALIDATE_STALLED:
      "The playback check stalled for {seconds} s and was stopped. File: {out_path}",
    CONVERT_VALIDATE_NO_FFMPEG:
      "There is nothing to check playback with: the bundled FFmpeg does not work.",
    CONVERT_NO_ENCODER:
      "There is nothing to encode with: the bundled build has neither a hardware H.264 encoder nor a software one.",
    PLAN_NO_AUDIO_TRACKS:
      "The file has no audio track at all. Check that this is the right file: video without sound does not go into service.",
    PLAN_NO_SUCH_TRACK:
      "There is no audio track {number} in the file — there are {available} in all.",
    PLAN_HEIGHT_ZERO: "The frame height cannot be zero.",
    PLAN_HEIGHT_ABOVE_SOURCE: "Height {asked} is above the source ({source}).",
    PLAN_BITRATE_ZERO: "The target bitrate cannot be zero.",
    PLAN_BITRATE_ABOVE_SOURCE: "{asked_kbps} kbit/s is above the source ({source_kbps} kbit/s).",

    // --- how a long task can end badly ---
    CONVERT_VALIDATION_FAILED: "{problems} The file was left where it is: {out_path}",
    UPLOAD_SHORT:
      "{sent|bytes} of {total|bytes} reached the server. The file was not put into service.",
    UPLOAD_CHECKSUM_MISMATCH:
      "The transferred file differs from the source. It was not put into service and the temporary data was cleared away — start the upload again.",
    UPLOAD_SOURCE_CHANGED:
      "The source file has changed since the transfer began. Continuing would splice together two different versions — start the upload again.",
    UPLOAD_SOURCE_UNREADABLE: "The source file is unavailable: {path}",
    UPLOAD_TOO_MANY_BREAKS:
      "The transfer broke off {attempts} {attempts|plural:time} in a row. Check the connection and resume the task.",

    // --- stages of a long task ---
    STAGE_CONVERTING: "preparing the file",
    STAGE_VALIDATING: "checking playback",
    STAGE_CHECKSUM: "comparing checksums",
    STAGE_MEASURING_QUALITY: "measuring quality on the material itself",
    STAGE_PREPARING_MEASUREMENT: "preparing the measurement: reading the film and trial-encoding",
    STAGE_BUILDING_LADDER: "preparing the variants",
    STAGE_SENDING_VARIANT: "sending a variant to the server",
    STAGE_CUTTING_SEGMENTS: "cutting into segments — on the server",
    STAGE_VERIFYING_LADDER: "checking that every variant is served",
    STAGE_STOP_UNCONFIRMED: "stopping on the server — not confirmed yet, trying again",
    STAGE_STOPPING_AFTER_STEP: "stopping — waiting for the current command on the server to finish",
    STAGE_DEPLOYING: "Setting the server up",
    STAGE_WAITING_UNHEARD_COMMAND: "Waiting for an interrupted command on the server to finish",
    STAGE_DONE: "done",

    // --- what closing the application would do ---
    ON_CLOSE_RESUMES_FROM: "will continue from {percent}% at the next start",
    ON_CLOSE_RESTARTS_LOSING: "will have to start over — {percent}% of the work would be lost",
    ON_CLOSE_NOT_STARTED_YET: "has not begun yet, will start later",
    ON_CLOSE_MUST_RUN_AGAIN: "will have to be run again",
    ON_CLOSE_WORK_KEPT_START_AGAIN:
      "the {percent}% already done will keep, but you will have to start it again — it does not come back by itself",

    // --- steps of the connection check ---
    STEP_NET_BANNER: "answers with {banner}",
    STEP_NET_TIMEOUT: "the server did not answer within {seconds} s",
    STEP_NET_SILENT_CLOSED:
      "the connection was accepted and closed at once: SSH does not answer on this port",
    STEP_NET_SILENT: "connection accepted, but SSH is silent — check the port number",
    STEP_NET_NOT_SSH: "something other than SSH answers on port {port}: “{got}”",
    STEP_LOGIN_FINGERPRINT_UNCONFIRMED: "fingerprint not confirmed",
    STEP_LOGIN_OK: "signed in as {user}",
    STEP_VIDEO_DIR_OK: "{dir} is readable and writable",
    STEP_VIDEO_DIR_MISSING_OR_DENIED:
      "the directory {dir} does not exist, or the user {user} has no permission for it",
    STEP_DOMAIN_OK_NO_FILES:
      "{domain} answers over HTTPS (code {code}); there are no files in the directory, so serving itself cannot be checked yet",
    STEP_DOMAIN_FILE_NOT_SERVED: "the domain answers, but the file is not served: {url} → {code}",
    STEP_DOMAIN_OK: "files are being served: checked on {url}",
    STEP_DOMAIN_EMPTY_BODY: "{url} returned code {code}, but the body was empty",
    STEP_DOMAIN_TIMEOUT: "{domain} did not answer within {seconds} s",
    STEP_DOMAIN_NO_CONNECTION: "no connection to {domain} — check the domain record",
    SYSTEM_ERROR: "{text}",

    // --- why a stream cannot simply be carried across ---
    REASON_VIDEO_NOT_H264: "video is {codec} — the VRChat player only plays H.264",
    REASON_VIDEO_PIX_FMT:
      "video is H.264 but in {pix_fmt} rather than yuv420p — a strict player will not take it",
    REASON_TONEMAP: "the source is in high dynamic range and has to be brought down to ordinary",
    REASON_RESIZE: "the frame size is changing",
    REASON_KEYFRAMES_UNALIGNED:
      "The source's keyframes do not fall where the segment boundaries will",
    REASON_TARGET_BITRATE: "a target bitrate was set",
    REASON_AUDIO_NOT_AAC: "audio is {codec} — the target format is AAC",
    REASON_AUDIO_PROFILE:
      "the audio is AAC but the profile is {profile} — the target is AAC-LC, and other profiles do not play for everyone",
    REASON_AUDIO_PROFILE_UNKNOWN:
      "the container does not say which audio profile this is — re-encoded to be certain it is AAC-LC",
    REASON_AUDIO_CHANNELS: "audio has {channels} channels — the target format is stereo",
    REASON_AUDIO_TOO_FAT: "the track is fatter than the target bitrate",

    // --- what to say about the choice of encoder ---
    NOTICE_PROBE_UNCALIBRATED:
      "The complexity probe did not run on NVIDIA — the top rung may be off. For an important film, run a full measurement.",
    NOTICE_PROBE_FAILED:
      "Measuring failed — the top rung comes from a constant. Better adjust the rungs.",
    NOTICE_MEASUREMENT_BORROWED: "Rungs taken from the measurement of {from}.",
    NOTICE_MEASUREMENT_PARTIAL: "Points measured: {measured} of {total}.",
    NOTICE_VARIANTS_REUSED: "Ready variants on the server: {count} — not rebuilt.",
    NOTICE_REENCODED_FOR_KEYFRAMES:
      "The rung was re-encoded so its keyframes line up with the others.",
    WARN_LIMIT_FOLLOWS_THE_ADDRESS: "The limit is put on an address, not on a person.",
    WARN_ADDRESS_SHARED: "{count} viewers are on this address — the limit reaches all of them.",
    WARN_CAP_BELOW_LIGHTEST:
      "The cap is below the lightest rung ({lightest_bps} bit/s) — the viewer gets that rung.",
    LIMITS_ROLLBACK_UNSUCCESSFUL:
      "Putting the previous limits back failed — the serving may not be working. Check Diagnostics.",
    LIMITS_ROLLBACK_NOT_STARTED:
      "The previous limits were not put back: the end of the previous command was not confirmed. Reload the list in a minute. If the change got as far as replacing the file, the new rules may have stayed.",
    NOTICE_NO_HARDWARE_FOUND:
      "No acceleration — the processor encodes. Quality will not suffer, but it takes several times longer.",
    NOTICE_SOFTWARE_AS_ASKED: "The processor encodes, as you asked.",
    NOTICE_HARDWARE_FAILED:
      "Acceleration via {encoder|encoder} failed — the processor encodes. Quality will not suffer; it takes longer.",

    // --- transfer ---
    UPLOAD_FILE_UNREADABLE: "The file was not found, or cannot be read.",
    UPLOAD_NOT_A_FILE: "What was given is not a file.",
    UPLOAD_NAME_EMPTY: "Enter the name the file will be visible under to viewers.",
    UPLOAD_ALREADY_RUNNING:
      "The file “{name}” is already being uploaded to this server. Wait for it to finish, or cancel that task.",
    BUILD_ALREADY_RUNNING:
      "A build of the set “{slug}” is already running on this server. Wait for it to finish.",
    UPLOAD_NAME_RESERVED: "That name belongs to an internal serving entry — choose another.",
    DOMAIN_ADD_RECORD:
      "Add a {record} record for “{name}” with the value {value} at your registrar.",
    DOMAIN_FIX_RECORD: "The {record} record for “{name}” leads to {to} — change it to {value}.",
    DOMAIN_REMOVE_RECORD:
      "Remove the {record} record for “{name}” (leads to {to}): IPv6 will be off.",
    DOMAIN_SERVER_HAS_NO_IPV6:
      "The server has no IPv6, but the AAAA record for “{name}” leads to {to} — remove it.",
    CHANGE_LOOKS_ONLY: "only looks; changes nothing",
    CHANGE_INSTALLS_PACKAGES: "installs {count|plural:package}: {names}",
    CHANGE_CREATES_SWAP_FILE: "creates a {megabytes} MB swap file",
    CHANGE_CREATES_SYSTEM_USER: "creates the system user \u201c{name}\u201d",
    CHANGE_CREATES_DIRECTORY: "creates the directory {path}",
    CHANGE_WRITES_FILE: "writes the file {path}",
    CHANGE_ENABLES_SERVICE: "enables the \u201c{name}\u201d service and starts it",
    CHANGE_OPENS_PORTS: "opens {count|plural:port} to the outside: {ports}",
    CHANGE_CLOSES_EVERYTHING_ELSE: "closes every other port to the outside",
    CHANGE_ADDS_SSH_KEY: "adds the application\u2019s key to authorized_keys",
    CHANGE_TURNS_PASSWORD_LOGIN_OFF: "turns password logins off \u2014 only the key will work",
    CHANGE_TURNS_IPV6_OFF: "turns IPv6 off",
    CHANGE_SETS_KERNEL_SETTINGS: "changes the kernel\u2019s network and disk settings",
    DEPLOY_STOPPED_AT_STEP:
      "It stopped at the step \u201c{step|deployStep}\u201d. Steps completed: {done}.",
    DEPLOY_STOPPED_AFTER: "Steps completed: {done}.",
    NOTICE_CANCELLED_AFTER_PUBLISH:
      "“{name}” was already published. If not needed, delete it in the library.",
    NOTICE_NOT_FILED_UNDER_MEDIUM: "“{name}” is uploaded but sits in Unrecognized.",
    NOTICE_LEFTOVER_PENDING: "The partial “{name}” will be removed from the server later.",
    NOTICE_LEFTOVER_REMOVED:
      "The partly uploaded “{name}” left by the cancellation has been removed from the server.",
    LADDER_NOT_ENOUGH_SPACE:
      "Will not fit: ~{needed|bytes} needed, {free|bytes} free, {short_by|bytes} short. Rungs: {rungs} — not all need building.",
    OBJECTION_RUNG_ABOVE_SOURCE:
      "Rung {index}: above the source — those bits add nothing but weight",
    OBJECTION_BUFSIZE_TOO_LARGE:
      "Rung {index}: the buffer is larger than the ceiling — real peaks will exceed it and a viewer will stall",
    OBJECTION_LEVEL_EXCEEDED:
      "Rung {index}: the variant does not fit the level it declares, {level}",
    OBJECTION_OUT_OF_ORDER: "Rung {index}: the rungs are not in descending order",
    OBJECTION_BAD_STEP: "Rung {index}: {times} times the one below",
    CHAIN_STOPPED_BY_OBJECTION:
      "Build not queued: the rungs have objections. The rest of the queue carries on.",
    NOTICE_CHECK_POINT_RUNNING: "Checking the borrowed measurement — under a minute.",
    STAGE_CHECKING_LOAN: "Checking the borrowed measurement",
    CHECK_POINT_NOT_COMPARABLE:
      "Loan withdrawn: at {bitrate} Mbit/s, {height}p only {used} of {asked} chunks measured. Check the file.",
    CHECK_POINT_APART:
      "At {bitrate} Mbit/s, {height}p: donor {donor} VMAF, this one {borrower} — {apart} hundredths apart, threshold 100.",
    NOTICE_CHECK_POINT_HELD:
      "Loan confirmed: {bitrate} Mbit/s, {height}p — {apart} hundredths of VMAF apart, threshold 100.",
    NOTICE_MEASUREMENT_THIN:
      "Incomplete points: {points}. E.g. {bitrate} Mbit/s, {height}p: {used} of {asked} chunks. Check the file.",
    NOTICE_MATERIAL_APART:
      "Apart from the donor: median {median}%, heavy scenes {p90}%, peak to median {ratio}%. No threshold — if large, measure it yourself.",
    NOTICE_VARIANTS_STRANDED:
      "Rungs left outside the set on the server: {count} ({names}). Viewers do not get them; they take space.",
    LEND_FRAME_DIFFERS: "A different frame size.",
    LEND_FPS_DIFFERS: "A different frame rate.",
    LEND_NATIVE_HEIGHT_DIFFERS: "A different native height (upscale).",
    LEND_CODEC_DIFFERS: "Different source codecs.",
    LEND_PIXEL_FORMAT_DIFFERS: "A different pixel format (8/10 bit).",
    LEND_COLOUR_TRANSFER_DIFFERS: "Different transfer curves (SDR/HDR).",
    LEND_TOO_SHORT: "The film is shorter than the donor's measured chunks.",
    LEND_MATERIAL_NOT_KNOWN:
      "It is not recorded what material that measurement was made on — measure again.",
    LADDER_NO_ROOM_HERE:
      "No room for a variant: {needed} bytes needed, {free} free, {short_by} short. Folder: {at}. Free space or change the folder in settings.",
    LADDER_SPACE_UNKNOWN:
      "How much room the set would take could not be worked out, so the build is going ahead without that check.",
    NOT_ENOUGH_SPACE:
      "The server is {short_by|bytes} short — {needed|bytes} needed, {free|bytes} free.",
    NAME_WILL_BE_REPLACED: "The file “{name}” is already being served — it will be replaced.",
    CDN_KEEPS_OLD_COPY:
      "The CDN will keep the previous copy for a while, and viewers will get the old one.",
    VIEWERS_ACTIVE_UPLOAD: "Open connections: {connections}. The upload may stall playback.",

    // The state of the server (FR-070). Every reading carries the figures it rests on.
    HEALTH_NOT_ESTABLISHED: "Could not be established.",
    HEALTH_NOT_IN_CONTAINER:
      "Not visible inside a container: the kernel settings and the disk belong to the host.",
    HEALTH_SERVING_RUNNING: "The serving is running.",
    HEALTH_SERVING_STOPPED:
      "The serving service «{service}» is not running: {state}. Viewers will get nothing right now.",
    HEALTH_DELIVERY_OK: "The server answers over HTTPS and understands a range request ({status}).",
    HEALTH_DELIVERY_NO_RANGES:
      "The server sent the whole file instead of the range asked for ({status}). Watching works, seeking does not.",
    HEALTH_DELIVERY_REFUSED: "The server answered {status} to its own check.",
    HEALTH_DELIVERY_SILENT: "The server did not answer over HTTPS.",
    HEALTH_NOTHING_TO_SERVE: "Nothing to check: there is no video on the server yet.",
    HEALTH_FIREWALL_ON: "The firewall is on.",
    HEALTH_FIREWALL_OFF:
      "The firewall is off: {status}. Everything that listens is open to the outside.",
    HEALTH_OPEN_PORTS: "Ports open to the outside: {count} — {ports}.",
    HEALTH_MEMORY: "Memory: {used_mb} MB used of {total_mb}.",
    HEALTH_CACHE_IDLE:
      "The serving cache holds {cache_mb} MB. Nobody is watching, so there is nothing to fill it with.",
    HEALTH_CACHE_SMALL:
      "The serving cache holds only {cache_mb} MB of {total_mb} while {watching} are watching. So it is being served off the disk rather than out of memory.",
    HEALTH_CACHE_OK:
      "The serving cache holds {cache_mb} MB, {watching} watching. Served out of memory.",
    HEALTH_NO_SWAP:
      "There is no swap at all, and {total_mb} MB of memory. At the peak of an install that may not be enough.",
    HEALTH_SWAP_IN_USE: "{used_mb} MB of {total_mb} in swap. Memory is short.",
    HEALTH_SWAP_OK: "Swap is barely touched: {used_mb} MB of {total_mb}.",
    HEALTH_DISK: "{free_mb} MB free of {total_mb} on the disk.",
    HEALTH_NETWORK_TUNED: "The network is tuned: {congestion}.",
    HEALTH_NETWORK_UNTUNED:
      "Network: {congestion}/{qdisc} instead of {wanted_congestion}/{wanted_qdisc} — slower.",
    HEALTH_READAHEAD_OK: "The disk's readahead is {kb} KB.",
    HEALTH_READAHEAD_SMALL: "Disk readahead {kb} KB instead of {wanted_kb}.",
    HEALTH_NO_AUTO_RESTART: "The serving does not restart itself after a crash.",
    HEALTH_AUTO_RESTART: "The serving comes back on its own: {mode}.",

    // Why the picture stops (FR-072). The conclusion is sometimes wrong, and has to be arguable.
    STALLS_TOO_SHORT: "Too short a stretch — {seconds} s. There is nothing to judge by.",
    STALLS_KEEPING_UP: "The viewer keeps up: {ratio}× real time, link {mbit_s} Mbit/s.",
    STALLS_SERVER_LINK:
      "The server's own link is the limit: {out_mbit_s} Mbit/s going out of {capacity_mbit_s} possible.",
    STALLS_DISK: "The disk is the limit: {disk_read_mb_s} MB/s read, {ratio}× real time received.",
    STALLS_FILE_PEAKS:
      "The file's peaks: link {mbit_s} Mbit/s, average {average_mbit}, 10 s peak {peak_10s_mbit}.",
    STALLS_THE_PLAYER:
      "The player, not the link: {in_download_mbit_s} Mbit/s against {average_mbit} needed; by the clock {mbit_s} Mbit/s, ratio {ratio}. Restarts: {restarts}, skipped: {skipped}.",
    STALLS_VIEWER_LINK:
      "The viewer's link is short: {ratio}× at {mbit_s} Mbit/s (in downloads {in_download_mbit_s}). Skipped: {skipped}, restarts: {restarts}.",
    // --- videos in work (T672) ---
    VIDEO_ALREADY_LISTED: "This video is already on the list",
    RUNG_FILE_CLAIMED:
      "The file “{name}” belongs to a medium and is not overwritten. Change the rungs or the name.",
    OLD_SET_UNRECOGNIZED:
      "“{name}” is on the server — a set nobody owns under this name. “Replace” removes it.",
    CONFIRM_STOP_SERVER_WORK: "Stop {count} {count|plural:taskAcc} on this server and delete it?",
  },

  plurals: {
    file: { one: "file", few: "files", many: "files" },
    media: { one: "medium", few: "media", many: "media" },
    time: { one: "time", few: "times", many: "times" },
    task: { one: "task", few: "tasks", many: "tasks" },
    taskAcc: { one: "task", few: "tasks", many: "tasks" },
    track: { one: "track", few: "tracks", many: "tracks" },
    package: { one: "package", few: "packages", many: "packages" },
    port: { one: "port", few: "ports", many: "ports" },
  },

  ui: {
    common: {
      dismiss: "Dismiss",
      more: "Details",
      cancel: "Cancel",
      close: "Close",
      refresh: "Refresh",
      loading: "Loading…",
      nothing: "—",
      language: "Language",
      theme: { light: "Light", dark: "Dark", system: "Follow the system" },
    },

    ladder: {
      columnBuild: "Build",
      buildThisRung: "Build the {mbps} Mbit/s rung",
      rungs: "Rungs",
      columnBitrate: "Bitrate",
      columnSize: "Frame",
      columnQuality: "Quality",
      columnWhy: "Why",
      reasons: {
        probed_anchor: "Top: above {mbps} Mbit/s nobody sees a difference.",
        capped_by_source: "Cut to {mbps} Mbit/s — all the source has.",
        capped_by_upscale: "Cut to {mbps} Mbit/s — above it is upscale.",
        step_down: "Step down: {mbps} Mbit/s, {times} times less.",
        fallback_constant: "Not measured: {mbps} Mbit/s from a constant.",
        lowered_for_density: "Height {height}: at {mbps} Mbit/s a full frame breaks up.",
        full_resolution: "Full frame {width}×{height} — {mbps} Mbit/s is enough.",
        single_rung_only: "One rung, {mbps} Mbit/s — a second would look the same.",
        measured_optimum: "Height {height} — best measured VMAF at {mbps} Mbit/s.",
        borrowed_measurement: "Measured on another file.",
        filled_a_gap: "{mbps} Mbit/s — fills a step that was too big.",
        edited_by_hand: "{mbps} Mbit/s typed in by hand, not measured.",
      },
      notMeasured: "not measured",
      vmafIs: "VMAF {value}",
      vmafBorrowed: "VMAF {value}, from another file",
      objections: "Objections",
    },

    serverState: {
      title: "The server’s state",
      asking: "Looking at what this server is…",
      clean: "Not deployed.",
      deployIt: "Set it up",
      unfinished: "Deployment not finished.",
      finishIt: "Finish it",
      versions: (server: number, app: number) => `Server version: ${server}, app version: ${app}.`,
      tooNew: "The server side is newer than the app — read only.",
      updateIt: "Update the server side",
      foreign: "Someone else's setup — the app leaves it alone.",
      unreachable: "The server did not answer. Showing the last known state.",
    },
    deploy: {
      title: "Set the server up",
      willChange: "What will be done",
      agreeAndStart: "Agreed — set it up",
      running: "Deploying. You can close this screen.",
      finished: "The server is deployed.",
      machine: (memoryMb: number, disk: string) => `Memory ${memoryMb} MB, disk ${disk}.`,

      ipv6Question: "What should happen to IPv6 on this server?",
      ipv6NotChosen: "Choose one.",
      ipv6Keep: "Keep IPv6",
      ipv6KeepMeans: "Needs an AAAA record for the server's IPv6.",
      ipv6Disable: "Disable IPv6",
      ipv6DisableMeans: "Remove the domain's AAAA record.",

      domainTitle: "The domain record",
      domainAsking: "Asking the servers that hold the zone…",
      domainOk: "The domain points at this server.",
      domainNotPointed: "The domain does not lead here. Add an A record at your registrar.",
      domainSpreadsSlowly: "A record takes a few minutes to spread.",
      domainAskAgain: "Ask again",

      stepApplied: "done",
      stepToDo: "will be done",
      stepFailed: "failed",
      stepNotNeeded: "not needed on this server",
      stepNotHere: "cannot be established here",

      foreignCaddyfile:
        "The server has someone else's Caddyfile. Without consent, deployment stops.",
      replaceCaddyfile: "Replace it (a copy is kept)",
      replaceCaddyfileMeans:
        'Copy: /etc/vrcast/backup/<time>/Caddyfile. "Put it back" restores it only if the deployment finishes; otherwise restore it by hand from the copy.',
    },

    deploySteps: {
      DnsCheck: "Check the domain record",
      Swap: "Make a swap file",
      Packages: "Install the packages",
      UserDirs: "Create the user and the directories",
      Configs: "Write the serving configuration",
      Services: "Start the serving",
      SshKey: "Put the key in place",
      SshHardening: "Turn password logins off",
      Firewall: "Close everything not needed",
      Ipv6: "Carry out the IPv6 choice",
      Fail2ban: "Turn password guessing away",
      UnattendedUpgrades: "Turn on automatic security updates",
      Tuning: "Tune the network and the disk for serving",
      Verify: "Check the serving over the domain",
      State: "Write the server-side version",
    },

    forget: {
      title: "Remove my data",
      means: "Settings, server profiles, cache and secrets. Videos on the server stay.",
      dataDir: "Directory",
      servers: "Server profiles",
      secrets: "Secrets in the system store",
      none: "none",
      lockedOut: (names: string) =>
        `Will become unreachable for good: ${names} — the key exists only here.`,
      lockedOutAdvice: "Save the key to a file before removing.",
      agree: "I understand this cannot be undone",
      remove: "Remove everything",
      removing: "Removing…",
      done: "Removed. The application can be uninstalled.",
      secretsLeft: (names: string) =>
        `The system store would not give up these secrets: ${names}. They will have to be cleared by hand.`,
      dirLeft: "The data folder could not be removed — a file is in use.",
      tasksRunning: "Stop the tasks first.",
      changed: "The list changed — the agreement has been withdrawn, check again.",
      reading: "Reading the list again…",
    },
    update: {
      title: "Updates",
      installed: "Version installed",
      check: "Check for updates",
      checking: "Checking…",
      upToDate: "Nothing newer.",
      notConfigured: "Updates are not set up in this build.",
      unpackaged: "A build from source — nothing to update.",
      available: (version: string) => `Version ${version} is out.`,
      published: "Published",
      notes: "What is in it",
      install: "Update",
      installing: "Installing…",
      warnWindows: "The installer closes the app — start it again afterwards.",
      warnPackage:
        "The system will ask for the administrator password. The new version starts next launch.",
      warnAppImage: "The new version starts next launch.",
      // Neutral, because the particulars differ: being stopped, an administrator password,
      // a rewritten file — each is named by the warning right above the checkbox. One
      // wording covering all three would be true on exactly one platform.
      agree: "Install the new version",
      doneRestartLater: "Update installed. The new version starts next launch.",
    },
    appearance: {
      title: "Appearance",
      theme: "Theme",
      language: "Language",

      closeTitle: "The close button",
      closeToTray: "Minimise to the notification area",
      closeHides:
        "The window goes to the tray; tasks carry on. To quit, use Quit in the icon's menu.",
      closeExits: "The window closes and the app quits: this desktop has no tray.",
      closeUnknown: "Whether there is anywhere to minimise to could not be determined.",
      workDir: "Working files",
      workDirDefault: "Beside the source file",
      workDirPick: "Choose a folder",
      workDirReset: "Back to the default",
      workDirLeft: "{files} files ({mb} MB) were left in the old folder — remove them yourself.",

      mascot: "Mascot",
      mascotOn: "Show the mascot",
      animations: "Motion",
      animationsOn: "Smooth transitions",

      heavyTasks: "Concurrent heavy tasks",

      mascotIdle: "The mascot is resting",
      mascotWorking: "The mascot is busy working",
      mascotSuccess: "The mascot is pleased: it worked",
      mascotTrouble: "The mascot is worried: something did not work",
      mascotViewerTrouble: "The mascot is worried: a viewer is struggling",
    },
    diag: {
      title: "Diagnosis",
      period: "Over the last",
      minutes: "minutes",
      refresh: "Ask again",
      notDetermined: "Could not be determined",
      asking: "Asking the server…",

      healthTitle: "The state of the server",
      ratingFine: "fine",
      ratingWatch: "worth a look",
      ratingTrouble: "trouble",
      ratingUnknown: "not established",
      rawTitle: "What was actually read",
      readingServing: "The serving",
      readingDelivery: "Delivery over HTTPS",
      readingFirewall: "Firewall",
      readingOpenPorts: "Open ports",
      readingMemory: "Memory",
      readingServingCache: "Serving cache",
      readingSwap: "Swap",
      readingDiskSpace: "Disk space",
      readingNetwork: "Network settings",
      readingReadahead: "Readahead",
      readingAutoRestart: "Automatic restart",

      logsTitle: "The serving's log",
      logsNothing: "The serving wrote nothing down over this stretch.",
      logsRequests: (n: number) => `Requests: ${n}`,
      logsAddresses: (n: number) => `Addresses: ${n}`,
      logsUnreadable: (n: number) => `Lines that yielded nothing: ${n}`,
      logsCodes: "Answers",
      logsRangesOk: "Ranges are being served — 206 dominates, as it should.",
      logsRangesBad: "Files are sent whole — seeking does not work.",
      logsTopPaths: "Asked for most often",
      logsTopAddresses: "Asked most often",
      logsFailures: "Failures",
      logsNoFailures: "No failures.",
      logsLong: "Long requests",
      logsLongNormal: "Only long and nearly empty ones are flagged.",
      logsCapped: "Not everything is shown — pick a shorter period.",

      stallsTitle: "Why the picture stops",
      stallsNoViewers: "Nobody was watching over this stretch.",
      stallsSetAside: "Not viewers",
      stallsOurOwnCheck: "the server's own address — these are our own checks",
      stallsTooLittle: (n: number) => `segments: ${n} — a cache, or just arrived`,
      stallsRatio: "Content received against real time",
      stallsLink: "The viewer's link",
      stallsInDownload: "inside the downloads",
      stallsSkipped: "Segments skipped",
      stallsRestarts: "Player restarts",
      stallsWatching: "Watching",
      stallsLoad: "What the server was doing",
      stallsLoadCpu: "Processor",
      stallsLoadDisk: "Read off the disk",
      stallsLoadOut: "Going out",
      stallsLoadCapacity: "of a possible",
      stallsCapacityUnknown:
        "the link's capacity was not established — so it is never named as the culprit",

      bitrateTitle: "The file's bitrate peaks",
      bitratePick: "Choose a file",
      bitrateAverage: "Average",
      bitrateMedian: "Median",
      bitratePeak1: "One-second peak",
      bitratePeak10: "Ten-second peak",
      bitrateAt: "at",
      bitrateWorst: "Where it is heaviest",
      bitratePeakOverAverage: (times: number) =>
        `The ten-second peak is ${times} times the average.`,
      bitrateAdvice: "Fixed by re-encoding with a peak cap.",
      bitrateEven: "The file is even — no need to re-encode.",
    },
    upgrade: {
      title: "Update the server side",
      fromTo: (from: number, to: number) => `Version ${from} → ${to}.`,
      willChange: "What will change",
      nothingToDo: "Everything is already in place — nothing to change.",
      willKeep: "What will be copied aside first",
      keepsVideosAndCatalogue: "Videos and the catalogue are not touched.",
      agreeAndUpgrade: "Agreed — update",
      rollBack: "Put it back as it was",
      cancel: "Cancel",
      rollBackTitle: "Restore the settings from the copy?",
      rollBackReturns: "The settings files come back from the copy made before the last run.",
      rollBackKeeps:
        "Not put back: new files (such as 99-vrcast-ipv6.conf), live state (sysctl, ufw, swap, BBR, fail2ban) and quality limits. Videos, the catalogue and keys are not touched.",
      rollBackConfirm: "Understood — put it back",
      rollBackDone: "The settings were restored from the copy.",
    },
    limits: {
      title: "Capping quality",
      pickMedia: "Which medium",
      cap: "Cap, Mbit/s",
      willGet: "The viewer will get:",
      apply: "Cap it",
      confirm: "Understood — cap it",
      cancel: "Cancel",
      noLadder: "This medium has no quality set.",
      previewing: "Working out what the viewer would be left with…",
      applying: "Capping…",

      listTitle: "Limits in force",
      listEmpty: "Nothing is capped.",
      columnWho: "Address",
      columnMedia: "Medium",
      columnCap: "Cap",
      columnSince: "Since",
      remove: "Lift",
      removing: "Lifting…",
    },

    viewers: {
      placesMissing: "No place tables — country and city are not shown.",
      placesStale: "The place tables from {month} are out of date.",
      placesFetch: "Download",
      placesFetching: "Downloading…",
      placesFailed: "The download did not go through — try again later.",
      noServer: "No server selected.",
      starting: "Starting to watch…",
      nobody: "Nobody is watching at the moment.",
      notKnown: "not determined",
      watchingUnknown: "what they are watching is not known yet",
      speedNotYet: "Not measured yet.",
      needs: "needs",
      fine: "fine",
      columnAddress: "Address",
      columnPlace: "From",
      columnWatching: "Watching",
      columnSpeed: "Speed",
      columnFor: "For",
      columnState: "State",
      problems: {
        slowLink: "not enough link",
        slowLinkHint: "The link is too slow for this quality — cap the quality.",
        retransmits: "a lossy link",
        retransmitsHint: "Poor connection on the viewer's side.",
        stalls: "the pulling has stopped",
        stallsHint: "No data flowing. If it lasts, the viewing has dropped.",
      },
      watchingNow: "watching now",
      reconnecting: "The connection to the server was lost — reconnecting…",
      reconnectingTry: "Attempt {n}.",
      staleAge: "List from {age} ago — it may have changed.",
      staleNever: "No list has come from the server yet.",
      stopped: "Watching stopped: cannot sign in to the server. Check the server.",
      restart: "Start again",
      ageSeconds: "{n} s",
      ageMinutes: "{n} min",
    },

    sections: {
      servers: "Servers",
      library: "Library",
      video: "Video",
      viewers: "Viewers",
      limits: "Limits",
      diagnostics: "Diagnostics",
      appearance: "Appearance",
      tasks: "Tasks",
    },

    tray: {
      show: "Show the window",
      quit: "Quit",
    },

    sidebar: {
      sections: "Sections",
      version: "version {version}",
      aboutTitle: "About and licence",
      notReady: "Not built yet",
    },

    wizard: {
      dialogLabel: "Setting up a server",
      heading: "New server",
      stepData: "Details",
      stepFingerprint: "Fingerprint",
      stepTest: "Check",
      importFound: "Earlier settings found:",
      importNeedsPassphrase: " Enter the key's passphrase yourself.",
      importApply: "Fill in",
      fieldName: "Name",
      fieldNamePlaceholder: "My server",
      fieldHost: "Address",
      fieldHostPlaceholder: "IP address or name",
      fieldPort: "Port",
      fieldDomain: "Serving domain",
      fieldUser: "User",
      fieldAuth: "Sign-in",
      authKey: "By key",
      authPassword: "By password",
      authManagedKey: "With the key made while deploying",
      authManagedKeyNote: "Password sign-in is off on the server.",
      pickKey: "Browse\u2026",
      fieldKeyPath: "Path to the private key",
      fieldPassphrase: "Key passphrase",
      fieldPassword: "Password",
      optional: "Optional",
      fieldVideoDir: "Video directory on the server",
      fieldVideoDirPlaceholder: "default",
      fieldCdn: "CDN address",
      fieldCdnPlaceholder: "none",
      checking: "Checking…",
      next: "Next",
      fingerprintLead: "Compare the fingerprint with your hosting panel.",
      abandon: "Give up",
      fingerprintOk: "The fingerprint is right",
      testAgain: "Check again",
      done: "Done",
      testRunning: "Checking the connection…",
      stepSkipped: "not checked: we stopped earlier",
    },

    // T673 — the "Video" screen: one place from a file to a link. Short labels only.
    video: {
      heading: "Video",
      add: "Add video",
      startAll: "Start all",
      pickFilter: "Video",
      noServer: "No server selected.",
      goToServers: "Go to servers",
      empty: "Nothing yet.",
      refusedDrop: "Hide refusals",
      planning: "Working out the plan…",
      startsAfterPlan: "Starts once planned",
      rungLine: "{height}p · {mbps} Mbit/s",
      rungToMeasure: "to measure",
      onServer: "On the server ≈ {bytes|bytes}",
      aboutMinutes: "{what} ≈ {n} min",
      encodeTime: "Encoding",
      measureAndEncodeTime: "Measuring and encoding",
      preliminary: "preliminary",
      shortServer: "{bytes|bytes} short on the server",
      shortLocal: "{bytes|bytes} short on this computer",
      nameTaken: "The name “{slug}” is taken",
      objections: "Objections to the rungs: {n}",
      audio: "Audio",
      trackFallback: "Track {n}",
      mono: "mono",
      stereo: "stereo",
      channels: "{n} ch.",
      trackDefault: " (default)",
      trackLine: "{base}, {channels}{main}",
      title: "Title",
      editTitle: "Change title",
      saveTitle: "Save",
      start: "Start",
      rungs: "Rungs",
      remove: "Remove",
      pause: "Pause",
      resume: "Resume",
      cancel: "Cancel",
      stopping: "Stopping…",
      cancelled: "Cancelled",
      paused: "Paused",
      queued: "Queued",
      retry: "Retry",
      buildAnyway: "Build anyway",
      replace: "Replace",
      replaceAsk: "The old set “{title}” will be removed and built again",
      replaceAnyway: "Replace anyway",
      stagesLabel: "Stages",
      stages: {
        measuring: "Measure",
        encoding: "Encode",
        uploading: "Upload",
        cutting: "Cut",
        verifying: "Check",
        done: "Done",
      },
      rungOf: "rung {k} of {n}",
      speed: "{bytes|bytes}/s",
      left: "{time} left",
      copy: "Copy",
      copyCdn: "Copy via CDN",
      copied: "Copied",
      copyFailed: "Could not copy",
      editorTitle: "Rungs: {title}",
      resetRungs: "Back to the plan",
      saveRungs: "Save",
    },

    library: {
      heading: "Library",
      reading: "Reading the library…",
      noActiveServer: "No server selected.",
      goToServers: "Go to servers",
      newMedia: "New medium",
      serverLine: "Server:",
      empty: "Empty so far.",
      mediaFacts: "{n} {n|plural:file} · {bytes|bytes}",
      hasLadder: " · quality ladder",
      missingOnServer: " · {n} not found on the server",
      shortName: "Short name:",
      ladders: "Quality ladders: {list}",
      laddersHeading: "Quality ladders",
      setFilesHeading: "The set's rung files",
      leftoverMp4: "Extra mp4 files — {bytes|bytes}",
      leftoverRemove: "Remove",
      renameMedia: "Rename",
      buildSet: "Build a set",
      setBuilding: "Set is building",
      setStopped: "Build stopped",
      openVideo: "Open Video",
      deleteMedia: "Delete the medium",
      diskFree: "Free",
      diskOf: "of",
      diskVideos: "video takes up {bytes|bytes}",
      diskLabel: "Disk space used on the server",
      staleTitle: "The server is out of reach right now",
      staleHint: "Showing the last known state. Changes wait for the connection.",
      staleRetry: "Try again",
      linkDead: "the link does not work",
      linkDeadTitle: "The file is not on the server",
      linkFromServer: "Link from the server",
      linkCopy: "Copy the link",
      linkViaCdn: "Link through the CDN",
      linkCopiedServer: "copied, from the server",
      linkCopiedCdn: "copied, through the CDN",
      linkCopyFailed: "copying did not work",
      resolution: "Resolution",
      duration: "Length",
      bitrate: "Average bitrate",
      video: "Video",
      audio: "Audio",
      faststartWarning: "Plays only after a full download — prepare it again.",
      missingWarning: "Not on the server — the link does not work.",
      deleteFile: "Delete the file",
      unrecognizedTitle: "Not recognised",
      unrecognizedCount: "{n} {n|plural:file} · {bytes|bytes}",
      unrecognizedNote: "Files outside any medium. Assign them or delete them.",
      suggestionNote: "These look related (nothing merged):",
      suggestionGroup: "— {n} files, {why}",
      groupReason: {
        SAME_DIRECTORY: "in one directory",
        BITRATE_VARIANTS: "bitrate variants of one file",
      },
      assignTo: "Assign to a medium",
      moveTo: "Move to",
      assignChoose: "— choose —",
      createHeading: "New medium",
      fieldTitle: "Title",
      fieldSlugOptional: "Short name (optional)",
      fieldSlugPlaceholder: "made from the title",
      slugHint: "Latin letters, digits, - and _.",
      creating: "Creating…",
      create: "Create",
      renameHeading: "Rename “{title}”",
      fieldSlug: "Short name",
      slugChangeWarning: "Links already handed out will stop working.",
      renaming: "Renaming…",
      rename: "Rename",
      renameAnyway: "Rename anyway",
      deleteHeading: "Delete “{what}”?",
      deleteLabel: "Delete {what}",
      deleteNo: "Do not delete",
      deleting: "Deleting…",
      deleteYes: "Delete",
    },

    servers: {
      heading: "Servers",
      reading: "Reading the server list…",
      add: "Add a server",
      empty: "No servers yet.",
      activeBadge: "active",
      makeActive: "Make active",
      domain: "Domain",
      videoDir: "Video directory",
      cdn: "CDN",
      fingerprintUnconfirmed: "Fingerprint not confirmed — cannot connect.",
      testing: "Checking…",
      test: "Check the connection",
      confirmRemoval: "Remove the profile and its saved password?",
      removeYes: "Yes, delete",
      stoppingWork: "Stopping the tasks…",
      remove: "Delete",
      steps: {
        network: "The server is reachable over the network",
        login: "Signing in to the server",
        video_dir: "The video directory is reachable",
        domain: "Serving answers on the domain",
      },
      stepStatus: { ok: "passed", failed: "failed", skipped: "not checked" },
      edit: "Edit",
      editHeading: "Edit the server “{name}”",
      editSecretHint: "Leave empty to keep the current one.",
      leaveMadeKeyForFileHint:
        "The deployment key will be removed. Enter the key's passphrase, if it has one.",
      leaveMadeKeyForPasswordHint:
        "The deployment key will be removed. Enter the server's password.",
      editAddressChanged: "The address changed — confirm the fingerprint again.",
      save: "Save",
      saving: "Saving…",
    },

    tasks: {
      states: {
        queued: "queued",
        running: "running",
        paused: "paused",
        completed: "finished",
        failed: "failed",
        cancelled: "cancelled",
      },
      notes: "Notes: {n}",
      batchStop: "Stop the whole batch",
      batchIs: "Batch: {films} video(s), {left} task(s) left.",
      batchStopped: "Stopped {n} task(s)",
      counts: "Running: {running}. Waiting: {queued}.",
      heading: "Tasks",
      reading: "Reading the task list…",
      empty: "No tasks.",
      speed: "{mbit} Mbit/s",
      etaHours: "~{h} h {m} min left",
      etaMinutes: "~{m} min left",
      etaSoon: "less than a minute left",
      pause: "Pause",
      resume: "Resume",
      stop: "Cancel",
      viewResult: "See it in the library",
      kinds: {
        probe: "examining the source",
        convert: "preparing the file",
        upload: "uploading to the server",
        measure_quality: "measuring quality on the material",
        build_ladder: "building the quality ladder",
        deploy: "deploying",
        upgrade_server: "updating the server",
        diagnose: "diagnostics",
      },
      queueHeading: "In the queue",
      moveUp: "Move up the queue",
      moveDown: "Move down the queue",
      closeLosing: "Closing now will lose some work",
      closeSafe: "Safe to close — work resumes on next start",
      leaveQuestion: "Leave the application?",
      leaveUnknown: "What will happen to the tasks is unknown.",
      leaveConfirm: "Leave",
      leaveCancel: "Stay",
    },

    notifications: {
      completed: "Task finished",
      hiddenTitle: "The app is running in the tray",
      hiddenBody: 'Tray icon → "Show the window". On Windows 11 it may be under the ^ arrow.',
      failed: "Task failed",
      lookInTasks: "Details are in Tasks.",
      done: {
        upload: "The file was uploaded and put into service.",
        convert: "The file is prepared and ready to upload.",
        measure_quality: "The quality of this material has been measured.",
        build_ladder: "The quality ladder is built.",
        deploy: "Serving is deployed.",
        upgrade_server: "The server side is updated.",
      },
    },

    about: {
      title: "About",
      tagline: "managing a streaming server: library, file preparation, uploading, viewers.",
      build: "Build {commit} of {date}",
      licenceHeading: "Licence and source code",
      licenceBody1a: "This application is distributed under the",
      licenceName: "GNU General Public License, version 3 or later",
      licenceBody1b:
        ". That means you are free to use it, study it, change it and pass it on — and that whoever receives a build from you has the same freedoms.",
      sourceLead: "The source code of",
      sourceThisVersion: "this very version",
      sourceTag: "tag",
      sourceAvailableAt: "is available at:",
      sourceMissing:
        "If the source is not there, write to us — we are obliged to provide it. That is not a courtesy but a condition of the licence.",
      thirdPartyHeading: "Third-party work in this package",
      thirdPartyBody:
        "The application includes third-party libraries and a full FFmpeg build, each with its own terms. The complete list with licence texts is in the file",
      thirdPartyBodyTail:
        "beside the source code; it is generated from the dependency tree at build time rather than written by hand — otherwise it would be out of date within a month.",
      thirdPartyLink: "List of third-party components",
      geoHeading: "Where a viewer is",
      geoBody:
        "Country, city and provider are worked out on your own computer from the IP-to-City Lite and IP-to-ASN Lite tables by DB-IP, available under",
      geoLicence: "Creative Commons Attribution 4.0",
      geoBodyTail:
        ". The tables are not in the installer — the application fetches a current one itself and refreshes it monthly. Your viewers addresses go nowhere in the process: the lookup happens here.",
      schemaVersion: "Local storage version: {schema}. This is needed when investigating trouble.",
    },
  },
};
