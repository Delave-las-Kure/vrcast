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
      hint: "Check the user name and the password or key. If the server only offers key-based sign-in, a password will not do — set up a key.",
    },
    SSH_UNREACHABLE: {
      message: "Could not reach the server",
      hint: "Check that the server is switched on, the address is right, and the port is reachable from outside.",
    },
    HOST_KEY_CHANGED: {
      message: "The server's fingerprint has changed",
      hint: "This happens after a server is rebuilt — in that case confirm the new fingerprint. If the server has not changed, do not connect: this may be an impersonation.",
    },
    HOST_KEY_UNCONFIRMED: {
      message: "The server's fingerprint has not been confirmed yet",
      hint: "Compare the fingerprint shown with the one your hosting provider gives, then confirm it.",
    },
    HOST_KEY_IS_CERTIFICATE: {
      message: "The server presented a certificate instead of a key",
      hint: "This application works with servers that present an ordinary key. Turn off host certificates on the server.",
    },
    KEY_NEEDS_PASSPHRASE: {
      message: "The key is protected by a passphrase",
      hint: "Enter the passphrase for this key.",
    },
    KEY_UNREADABLE: {
      message: "The key file could not be read",
      hint: "Check the path, and that this is the private key rather than its public half (the one ending in .pub).",
    },
    VIDEO_DIR_DENIED: {
      message: "No access to the video directory on the server",
      hint: "Make sure the path is right and that the user has permission for that directory.",
    },

    // --- domain ---
    DOMAIN_NOT_SERVING: {
      message: "The domain is not serving video",
      hint: "Nothing answers on the domain. Check the server's condition in the diagnostics section.",
    },
    DOMAIN_NOT_POINTED: {
      message: "The domain is not attached to the server",
      hint: "Create an A record for this name at your registrar, pointing at the server's address. The change takes a few minutes to spread across the network.",
    },
    DOMAIN_POINTS_ELSEWHERE: {
      message: "The domain leads to a different server",
      hint: "Correct the A record so that it points at this server's address.",
    },
    IPV6_MISMATCH: {
      message: "The IPv6 choice does not match the domain records",
      hint: "Either add an AAAA record for this server's IPv6 address, or choose to disable IPv6 during deployment.",
    },

    // --- server state and deployment ---
    SERVER_NEEDS_UPGRADE: {
      message: "The server side needs updating",
      hint: "The server is running a version older than this application works with. Update it from the server’s card — the files and the catalogue are kept.",
    },
    SERVER_FOREIGN: {
      message: "Something else is already serving from this server",
      hint: "The application does not touch other people's configuration. Use a clean server, or remove the other setup by hand.",
    },
    SERVER_TOO_NEW: {
      message: "The server side is newer than this application understands",
      hint: "Update the application: working with a server whose arrangements it does not understand is not safe.",
    },
    DEPLOY_STEP_FAILED: {
      message: "A deployment step did not go through",
      hint: "See which step it stopped at and run the deployment again — the steps that already succeeded will not be repeated.",
    },
    SWAP_FAILED: {
      message: "The swap file could not be created",
      hint: "Free up space on the server's disk: the swap file needs at least a gigabyte.",
    },

    // --- library ---
    SLUG_TAKEN: {
      message: "That name is already taken",
      hint: "Choose another name: this one belongs to a different medium.",
    },
    MANIFEST_CONFLICT: {
      message: "The catalogue was changed by another application",
      hint: "Another copy of the application is working with this server. Refresh the list and try again.",
    },
    FILE_MISSING_ON_SERVER: {
      message: "The file is no longer on the server",
      hint: "It was deleted outside the application. Refresh the library so the list matches what is really there.",
    },
    FILE_IN_USE: {
      message: "Someone is watching this file right now",
      hint: "Deleting or renaming it will cut their viewing short. Wait until they finish, or confirm deliberately.",
    },

    // --- preparing files ---
    FFMPEG_BROKEN: {
      message: "The video tool will not start",
      hint: "Reinstall the application: the video tool that ships with it is damaged.",
    },
    NO_AUDIO_TRACKS: {
      message: "The file has no audio track at all",
      hint: "Choose a different source: there is nothing to serve without sound.",
    },
    DECODE_VALIDATION_FAILED: {
      message: "The finished file failed the playback check",
      hint: "The file is damaged and not fit to serve. Try preparing it again from the source.",
    },
    NO_HW_ENCODER: {
      message: "Hardware acceleration is not available",
      hint: "Preparation will run on the processor — slower, but just as good. If acceleration ought to be there, close whatever has taken the graphics card.",
    },
    LOCAL_DISK_FULL: {
      message: "Not enough room on this computer's disk",
      hint: "Free up space on this computer and try again.",
    },

    // --- transfer ---
    REMOTE_DISK_FULL: {
      message: "Not enough room on the server's disk",
      hint: "Free up space on the server: delete media you no longer need from the library.",
    },
    CHECKSUM_MISMATCH: {
      message: "The transferred file differs from the source",
      hint: "The transfer was corrupted. The file was not put into service — start the upload again.",
    },
    VIEWERS_ACTIVE: {
      message: "Someone is watching right now",
      hint: "An upload will push what they are watching out of the server's memory and their playback will stall. Better to wait until they finish.",
    },
    NAME_EXISTS: {
      message: "A file with that name is already being served",
      hint: "Choose another name, or confirm the replacement. Remember that a cached copy at the CDN will keep serving the old one for a while.",
    },

    // --- quality ladders ---
    RUNG_ABOVE_SOURCE: {
      message: "The quality rung is higher than the source itself",
      hint: "Lower the rung: detail that is not in the source will not appear, and the file will only grow.",
    },
    BUFSIZE_TOO_LARGE: {
      message: "The buffer is too large for the chosen peak limit",
      hint: "Make the buffer roughly equal to the peak limit, or real peaks will exceed the limit and viewers will see stalls.",
    },
    LEVEL_EXCEEDED: {
      message: "The stream does not fit the chosen compatibility level",
      hint: "The level is judged by two limits — per frame and per second. Lower the bitrate, the frame rate, or the resolution.",
    },
    LADDER_INCOMPLETE: {
      message: "The quality ladder was not built in full",
      hint: "Some variants are not being served. Run the build again — the finished ones will not be rebuilt.",
    },
    VMAF_UNAVAILABLE: {
      message: "This build of FFmpeg cannot measure quality",
      hint: "A quality ladder is chosen by measuring the material rather than by formula, and without libvmaf there is nothing to measure with. Reinstall the application — its FFmpeg is built with libvmaf.",
    },
    LADDER_NOT_MEASURED: {
      message: "The quality of this material has not been measured yet",
      hint: "Rungs taken from the formula are a guess: on one film it asks for twice what is needed, on another it understates. Run the measurement, or borrow one from the first episode of the same season.",
    },
    MEASUREMENT_NOT_FOUND: {
      message: "There is no such measurement",
      hint: "It may have been deleted, or taken for a different target codec. Measurements do not carry between codecs: AV1's advantage over H.264 melts as the bitrate rises, and there is no constant multiplier.",
    },
    MEASUREMENT_DIFFERENT_MATERIAL: {
      message: "That measurement was taken on different material",
      hint: "Frame size, frame rate and the height the material really has must all agree. Native 4K and an upscale behave differently: the point where the resolution should drop sits somewhere else entirely.",
    },
    NO_LADDER_FOR_MEDIA: {
      message: "This medium has no quality ladder",
      hint: "Build a quality ladder first: capping quality means choosing from the rungs that exist.",
    },

    // --- web server configuration ---
    CADDY_VALIDATE_FAILED: {
      message: "The new server configuration turned out to be invalid",
      hint: "Nothing was applied and serving continues as before. Please report this error — it is a fault in the application.",
    },
    CADDY_RELOAD_FAILED: {
      message: "The server did not accept the new configuration",
      hint: "The previous configuration was restored and serving works. Check the server's condition in the diagnostics section.",
    },

    // --- tasks ---
    TASK_CANCELLED: {
      message: "The task was cancelled",
      hint: "Nothing to do: the task was stopped at your command.",
    },
    TASK_NOT_FOUND: {
      message: "Task not found",
      hint: "The task has already finished or been stopped. Refresh the task list.",
    },
    TASK_BAD_TRANSITION: {
      message: "The task is in a state this cannot be done from",
      hint: "Refresh the task list: their state has changed.",
    },
    TASK_NOT_PAUSABLE: {
      message: "A task of this kind cannot be paused",
      hint: "Short tasks are not paused — it is simpler to cancel one and run it again.",
    },

    // --- input and confirmation ---
    INVALID_INPUT: {
      message: "The details entered will not do",
      hint: "Correct the marked fields and try again. What exactly is wrong is in the message.",
    },
    CONFIRMATION_REQUIRED: {
      message: "Confirmation needed",
      hint: "Read what is about to happen, then confirm. There will be no undoing it.",
    },

    // --- updating the application itself ---
    UPDATE_CHECK_FAILED: {
      message: "Could not check for updates",
      hint: "Check the network connection and try again. The application carries on as before.",
    },
    UPDATE_INSTALL_FAILED: {
      message: "Could not install the update",
      hint: "Download the installer from the releases page and run it yourself. The copy you have is untouched.",
    },

    // --- everything else ---
    STORAGE_FAILED: {
      message: "Could not reach local storage",
      hint: "Check that there is room on the disk and that the application has permission for its own data directory.",
    },
    INTERNAL: {
      message: "An internal error in the application",
      hint: "Please report this error. If it keeps happening, the logs in the diagnostics section will help.",
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
    PROFILE_NOT_FOUND: "There is no such server — its profile may have been deleted.",
    FINGERPRINT_EMPTY: "The fingerprint is empty — there is nothing to confirm.",

    // --- domain field ---
    DOMAIN_EMPTY:
      "Enter the domain you serve from — without it there is no viewer link to hand out and no way to check that serving works.",
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
    VIDEO_DIR_AT_ROOT:
      "The serving directory is at the root of the file system — there is nowhere beside it to assemble a file, and assembling inside it is not allowed: a half-transferred file would become visible to viewers.",

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
      "Delete “{what}”? {files} {files|plural:file} will be removed, freeing {bytes|bytes}.",
    VIEWERS_ACTIVE_DELETE:
      "The server is serving data right now — {connections} connections are open. Deleting may cut someone's viewing short.",

    // --- preparing files ---
    FFMPEG_SELF_BROKEN:
      "The bundled FFmpeg does not work — there is nothing to prepare files with. Reinstall the application: an antivirus may have removed part of it.",
    FFMPEG_NO_X264:
      "The bundled FFmpeg was built without the software H.264 encoder. On a machine without a suitable graphics card there would be nothing to prepare files with.",
    PROBE_NO_VIDEO: "There is no video in this file — perhaps the wrong file was chosen.",
    PROBE_UNREADABLE: "The file could not be parsed: it is damaged, or it is not video.",
    CONVERT_NO_OUT_PATH: "Where to put the prepared file was not specified.",
    CONVERT_OUT_OVERWRITES_SOURCE:
      "The prepared file cannot be written over the source — the source would be lost for good.",
    CONVERT_VALIDATE_NO_FFMPEG:
      "There is nothing to check playback with: the bundled FFmpeg does not work.",
    CONVERT_NO_ENCODER:
      "There is nothing to encode with: the bundled build has neither a hardware H.264 encoder nor a software one.",
    PLAN_NO_AUDIO_TRACKS:
      "The file has no audio track at all. Check that this is the right file: video without sound does not go into service.",
    PLAN_NO_SUCH_TRACK:
      "There is no audio track {number} in the file — there are {available} in all.",
    PLAN_HEIGHT_ZERO: "The frame height cannot be zero.",
    PLAN_HEIGHT_ABOVE_SOURCE:
      "You are asking for {asked} lines where the source has {source}. The picture can be stretched, but no detail will appear from it — only the file and the time will grow.",
    PLAN_BITRATE_ZERO: "The target bitrate cannot be zero.",
    PLAN_BITRATE_ABOVE_SOURCE:
      "You are asking for {asked_kbps} kbit/s from a source at {source_kbps} kbit/s. Encoding above the source is pointless: detail that is not there will not be added, and space and bandwidth go to waste.",

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
    STAGE_BUILDING_LADDER: "preparing the variants",
    STAGE_CUTTING_SEGMENTS: "cutting into segments — on the server",
    STAGE_VERIFYING_LADDER: "checking that every variant is served",
    STAGE_DEPLOYING: "Setting the server up",
    STAGE_DONE: "done",

    // --- what closing the application would do ---
    ON_CLOSE_RESUMES_FROM: "will continue from {percent}% at the next start",
    ON_CLOSE_RESTARTS_LOSING: "will have to start over — {percent}% of the work would be lost",
    ON_CLOSE_NOT_STARTED_YET: "has not begun yet, will start later",
    ON_CLOSE_MUST_RUN_AGAIN: "will have to be run again",

    // --- steps of the connection check ---
    STEP_NET_BANNER: "answers with {banner}",
    STEP_NET_TIMEOUT: "the server did not answer within {seconds} s",
    STEP_NET_SILENT_CLOSED:
      "the connection was accepted and closed at once: SSH does not answer on this port",
    STEP_NET_SILENT:
      "connections are accepted, but SSH stays silent. Some hosting providers' attack protection behaves this way: it answers on any port, even one with nothing behind it. Check the port number — SSH may be listening on another",
    STEP_NET_NOT_SSH: "something other than SSH answers on port {port}: “{got}”",
    STEP_LOGIN_FINGERPRINT_UNCONFIRMED:
      "the server's fingerprint has not been confirmed yet — confirm it and the check will go on",
    STEP_LOGIN_OK: "signed in as {user}",
    STEP_VIDEO_DIR_OK: "{dir} is readable and writable",
    STEP_VIDEO_DIR_MISSING_OR_DENIED:
      "the directory {dir} does not exist, or the user {user} has no permission for it",
    STEP_DOMAIN_OK_NO_FILES:
      "{domain} answers over HTTPS (code {code}); there are no files in the directory, so serving itself cannot be checked yet",
    STEP_DOMAIN_FILE_NOT_SERVED:
      "the domain answers, but the file is not served: {url} returned code {code}. The file is on the server, so this is a serving configuration problem",
    STEP_DOMAIN_OK: "files are being served: checked on {url}",
    STEP_DOMAIN_EMPTY_BODY: "{url} returned code {code}, but the body was empty",
    STEP_DOMAIN_TIMEOUT: "{domain} did not answer within {seconds} s",
    STEP_DOMAIN_NO_CONNECTION:
      "could not connect to {domain}: check that the domain record leads to this server",
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
    REASON_AUDIO_CHANNELS: "audio has {channels} channels — the target format is stereo",
    REASON_AUDIO_TOO_FAT: "the track is fatter than the target bitrate",

    // --- what to say about the choice of encoder ---
    NOTICE_PROBE_UNCALIBRATED:
      "The complexity probe ran on something other than an NVIDIA card, and the quality setting it uses was calibrated for one. The ladder came out, but its top rung rests on a number taken with a different ruler: if you know this material, check it against what you know, and run a full measurement for anything that matters.",
    NOTICE_PROBE_FAILED:
      "The material could not be measured, so the top rung comes from the old constant. A constant knows nothing about the material: on animation it asks for three times what is needed, and on dense action it understates. The rungs are worth going over by hand.",
    NOTICE_MEASUREMENT_BORROWED:
      "These rungs come from the measurement of {from}, not from a measurement of this file. For the next episode of the same season that is usually right — it is the same source. If the material differs (a different upscale, a different frame rate), measure it separately.",
    NOTICE_MEASUREMENT_PARTIAL:
      "{measured} points of {total} were measured; the rest would not encode. The ladder is built from what there is, but where points are missing the optimum may have gone unfound.",
    NOTICE_VARIANTS_REUSED:
      "{count} variants were already on the server and were not made again. The application asked the server what is there rather than its own note about a previous run: a note outlives the thing it describes.",
    NOTICE_REENCODED_FOR_KEYFRAMES:
      "This rung could have been carried across without re-encoding — its quality needs no change. But segments can only be cut at a keyframe, and this source's keyframes sit differently from the other rungs': the boundaries would stop lining up, and a viewer changing quality would see it stall. So the rung is re-encoded after all — hours instead of minutes.",
    WARN_LIMIT_FOLLOWS_THE_ADDRESS:
      "A limit is put on an address, and an address is not a person. If this viewer's address changes, the limit stays on the old one and lands on whoever takes it next; and they come back to the full set under their new one.",
    WARN_ADDRESS_SHARED:
      "{count} viewers are watching from this address right now. That is ordinary for a household or an office: the limit reaches all of them, not only the one who is struggling.",
    WARN_CAP_BELOW_LIGHTEST:
      "The cap is below the lightest rung there is ({lightest_bps} bit/s). The viewer gets that rung anyway — an empty description would leave them with no video at all. If it is still too much, a lighter rung has to be built rather than chosen.",
    NOTICE_NO_HARDWARE_FOUND:
      "No hardware acceleration was found on this machine — the processor will do the encoding. Quality will not suffer, but it will take several times longer: reckon on an hour where a graphics card would take ten minutes.",
    NOTICE_SOFTWARE_AS_ASKED:
      "The processor is encoding, as you asked. It will take several times longer than with hardware acceleration.",
    NOTICE_HARDWARE_FAILED:
      "Acceleration through {encoder|encoder} did not work — the processor will do the encoding. Quality will not suffer, but it will take several times longer.",

    // --- transfer ---
    UPLOAD_FILE_UNREADABLE: "The file was not found, or cannot be read.",
    UPLOAD_NOT_A_FILE: "What was given is not a file.",
    UPLOAD_NAME_EMPTY: "Enter the name the file will be visible under to viewers.",
    UPLOAD_ALREADY_RUNNING:
      "The file “{name}” is already being uploaded to this server. Wait for it to finish, or cancel that task.",
    UPLOAD_NAME_RESERVED: "That name belongs to an internal serving entry — choose another.",
    DOMAIN_ADD_RECORD:
      "Create a {record} record for “{name}” with the value {value} at your domain registrar. The change takes a few minutes to spread; the check can be run again.",
    DOMAIN_FIX_RECORD:
      "The {record} record for “{name}” currently leads to {to}. Change it to {value} — most often this is a record left over from the domain’s previous life.",
    DOMAIN_REMOVE_RECORD:
      "The {record} record for “{name}” leads to {to}, and IPv6 is to be turned off. Remove it: otherwise the domain goes on promising an address that will stop answering, and some viewers will try it first.",
    DOMAIN_SERVER_HAS_NO_IPV6:
      "The server has no IPv6 address of its own, and the AAAA record for “{name}” leads to {to}. Wherever it leads, it is not this machine — remove it.",
    LADDER_NOT_ENOUGH_SPACE:
      "The set will not fit: about {needed|bytes} are needed and {free|bytes} are free, {short_by|bytes} short. The set has {rungs} rungs, and you need not build them all.",
    LADDER_NO_ROOM_HERE:
      "This machine has no room for one variant: {needed} bytes are needed, {free} are free, {short_by} short. Writing to {at}. Variants are made one at a time and removed as soon as they are sent, so one is all that is needed — and there is nowhere to put even that. Free some space, or choose another folder in the settings.",
    LADDER_SPACE_UNKNOWN:
      "How much room the set would take could not be worked out, so the build is going ahead without that check.",
    NOT_ENOUGH_SPACE:
      "The server is {short_by|bytes} short — {needed|bytes} needed, {free|bytes} free.",
    NAME_WILL_BE_REPLACED: "The file “{name}” is already being served — it will be replaced.",
    CDN_KEEPS_OLD_COPY:
      "The CDN will keep the previous copy for a while, and viewers will get the old one.",
    VIEWERS_ACTIVE_UPLOAD:
      "The server is serving data right now — {connections} connections are open. An upload will push what they are watching out of its memory and playback will stall.",

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
      "The network is not set the way it was measured: {congestion}/{qdisc} instead of {wanted_congestion}/{wanted_qdisc}. The serving works, only slower.",
    HEALTH_READAHEAD_OK: "The disk's readahead is {kb} KB.",
    HEALTH_READAHEAD_SMALL:
      "The disk's readahead is {kb} KB instead of {wanted_kb}. Measured, that is the difference between 17 and 40–60 MB/s.",
    HEALTH_NO_AUTO_RESTART:
      "The serving does not come back on its own: after a crash it will lie there until somebody notices.",
    HEALTH_AUTO_RESTART: "The serving comes back on its own: {mode}.",

    // Why the picture stops (FR-072). The conclusion is sometimes wrong, and has to be arguable.
    STALLS_TOO_SHORT: "Too short a stretch — {seconds} s. There is nothing to judge by.",
    STALLS_KEEPING_UP:
      "The viewer is keeping up: {ratio}× of real time received, link {mbit_s} Mbit/s. The gaps between their requests are a full buffer, not a stall.",
    STALLS_SERVER_LINK:
      "The server's own link is the limit: {out_mbit_s} Mbit/s going out of {capacity_mbit_s} possible.",
    STALLS_DISK:
      "{disk_read_mb_s} MB/s is being read off the disk, and {ratio}× of real time received. The viewers are spread along the timeline and their pieces do not fit in memory.",
    STALLS_FILE_PEAKS:
      "The viewer's {mbit_s} Mbit/s carries the average bitrate of {average_mbit} but not the ten-second peak of {peak_10s_mbit}. It is the file that hangs the player; a re-encode with the peaks capped is the cure.",
    STALLS_VIEWER_LINK:
      "The viewer's link is short: {ratio}× of real time received at {mbit_s} Mbit/s (inside the downloads — {in_download_mbit_s}). Segments skipped: {skipped}, player restarts: {restarts}.",
  },

  plurals: {
    file: { one: "file", few: "files", many: "files" },
    media: { one: "medium", few: "media", many: "media" },
    time: { one: "time", few: "times", many: "times" },
    task: { one: "task", few: "tasks", many: "tasks" },
    track: { one: "track", few: "tracks", many: "tracks" },
  },

  ui: {
    common: {
      dismiss: "Dismiss",
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
      handedFrom: "The file from the preparation: {path}",
      pickAnother: "Choose another",
      explain:
        "A quality set is several versions of one film. A viewer's player takes whichever their connection can hold.",
      pickFile: "Choose a source",
      working: "Working out the rungs — looking at what this material is…",
    noServer: "No server is chosen, so there is nowhere to build the set. Pick one under Servers.",
    measuring: "Measuring the source…",
      sourceIs: "Source: {width}×{height}, {fps} frames a second, {bitrate}",
      peakIs: "Peak: {peak} — a viewer's connection has to hold this, not the average",

      fromMeasured: "The rungs were chosen by measuring this material",
      fromBorrowed: "The rungs come from a measurement of another file",
      fromFormula: "The rungs come from the formula — a guess, not a measurement",
      formulaExplain:
        "The formula knows nothing about the material: on animation it asks for three times what is needed, on dense action it understates. Until it has been measured, the set cannot be built.",

      measureTitle: "Measuring quality",
      measureExplain:
        "The application encodes three short chunks — a light one, a middling one and a heavy one — at several qualities, and looks at what actually comes out.",
      measureTakes: "About {minutes} min: {points} points",
      measureTakesResume: "About {minutes} min: {points} points left of {total}",
      measureStandsOn: "What this estimate stands on",
      measureChunks: "Measured on three pieces of the film, at minute {starts}. Not consecutive and not from the start: the opening is usually easier than what follows.",
      measureAnchor: "The top of the grid is {mbps} Mbit/s: that is what the complexity probe asked for on this material.",
      measureNothingLeft: "The whole grid is measured — there is nothing left to run.",
      estimateFromThisMachine:
        "Estimated from your own earlier measurements: {points} points at {seconds} s each — {times} times the modelled speed.",
      estimateNotAsked:
        "Your earlier measurements could not be read, so this estimate comes from the model. That is not the same as having none: there may be some, and then this figure is off.",
      estimateFromModel:
        "Estimated from a measurement on the developer's machine — yours may differ. It corrects itself after your very first measured point.",
      measureStart: "Measure",
      measureRunning: "Measuring — you can close this: the work is not lost",

      setName: "What the set is called on the server",
      rungs: "Rungs",
      columnBitrate: "Bitrate",
      columnSize: "Frame",
      columnQuality: "Quality",
      columnWhy: "Why",

      // Why a rung looks the way it does (T418). The keys are the core's `Reason` variants,
      // and `every_reason_a_rung_can_give_has_words` holds them to it. Each wording carries
      // the number it is about: a bare "a step down" is true of every rung but the top and
      // explains none of them.
      reasons: {
        probed_anchor:
          "The top of the ladder: at {mbps} Mbit/s the material stops asking for more, and further bits go into what nobody can see.",
        capped_by_source:
          "The top was cut to {mbps} Mbit/s — that is all there is in the source. There is nothing above it to encode.",
        capped_by_upscale:
          "The top was cut to {mbps} Mbit/s: above it the picture is upscaled, and the extra bits go into stretched pixels.",
        step_down: "A step down from the one above: {mbps} Mbit/s, {times} times less.",
        fallback_constant:
          "The material could not be measured, so {mbps} Mbit/s comes from the old constant. The constant knows nothing about this material.",
        lowered_for_density:
          "The height was lowered to {height}: at {mbps} Mbit/s and the full frame there would be too few bits per pixel, and the picture would break into squares.",
        full_resolution:
          "The frame was left whole — {width}×{height}: at {mbps} Mbit/s the density holds.",
        single_rung_only:
          "The material is too light for a ladder: one rung, {mbps} Mbit/s. A second would differ from the first by less than anyone can see.",
        measured_optimum:
          "The height {height} was chosen by the measurement, not by the formula: at {mbps} Mbit/s it scored the best VMAF of those tried.",
        borrowed_measurement: "The measurement behind this rung was taken on another file.",
      },
      notMeasured: "not measured",
      vmafIs: "VMAF {value}",
      vmafBorrowed: "VMAF {value}, from another file",
      measuredTitle: "What was measured ({points} points)",
      measuredColumnVmaf: "VMAF",
      measuredColumnActual: "Came out at",
      measuredChosen: "taken into the ladder",
      droppedAbove:
        "Dropped as excessive: {list}. Past the quality target those bits are paid for by every viewer and seen by none.",

      build: "Build the set",
      buildBlocked: "Cannot be built: the rungs are not measured",
      buildBlockedEmpty: "Nothing to build: there are no rungs",
      building: "Building — this takes a while",
      objections: "Objections",
      objectionAboveSource: "Rung {index}: above the source — those bits add nothing but weight",
      objectionBufsize:
        "Rung {index}: the buffer is larger than the ceiling — real peaks will exceed it and a viewer will stall",
      objectionLevel: "Rung {index}: the variant does not fit the level it declares, {level}",
      objectionOrder: "Rung {index}: the rungs are not in descending order",
      objectionStep: "Rung {index}: {times} times the one below — too far {tooMuch}",
      stepTooBig: "apart",
      stepTooSmall: "together",
    },

    serverState: {
      title: "The server’s state",
      asking: "Looking at what this server is…",
      clean: "The server is bare: no serving is deployed on it.",
      deployIt: "Set it up",
      unfinished:
        "The setup did not finish: a deployment was interrupted. Running it again does not redo what is done.",
      finishIt: "Finish it",
      versions: (server: number, app: number) =>
        `Server side: version ${server}. This application deploys ${app}.`,
      tooNew:
        "The server side is newer than this application understands. Reading only for now: writing files where a newer layout does not keep them is how a working server is quietly broken.",
      updateIt: "Update the server side",
      foreign:
        "This server already has somebody else’s serving on it. The application changes nothing here.",
      unreachable: "The server did not answer. The last thing known is shown.",
    },
    deploy: {
      title: "Set the server up",
      willChange: "What will be done",
      agreeAndStart: "Agreed — set it up",
      running: "Setting it up. The screen may be closed; the work carries on.",
      finished: "The server is set up and serving video over its domain.",
      machine: (memoryMb: number, disk: string) =>
        `${memoryMb} MB of memory, system disk ${disk}. On a machine with little memory installing packages is killed without a swap file — one will be made.`,

      ipv6Question: "What should happen to IPv6 on this server?",
      ipv6Keep: "Keep it",
      ipv6KeepMeans:
        "The protection will cover IPv6 as fully as IPv4. The domain must then have an AAAA record pointing at this server’s IPv6 address — otherwise viewers whose connection prefers IPv6 go nowhere, and the complaint arrives not as “the server is broken” but as “it doesn’t open for me”.",
      ipv6Disable: "Turn it off",
      ipv6DisableMeans:
        "The serving will not answer over IPv6 at all. The domain must then have no AAAA record: left there, it goes on promising an address that has gone quiet.",

      domainTitle: "The domain record",
      domainAsking: "Asking the servers that hold the zone…",
      domainOk: "The domain points at this server.",
      domainNotPointed:
        "The domain is not attached to the server. Create a record at your registrar.",
      domainSpreadsSlowly:
        "The change takes a few minutes to spread. Once the record is created, ask again.",
      domainAskAgain: "Ask again",

      stepApplied: "done",
      stepToDo: "will be done",
      stepFailed: "failed",
      stepNotNeeded: "not needed on this server",
      stepNotHere: "cannot be established here",
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
      means:
        "Everything the application keeps about you: settings, server profiles, the library cache, the place tables and the secrets in the system store. The videos on your server are not touched.",
      dataDir: "Directory",
      servers: "Server profiles",
      secrets: "Secrets in the system store",
      none: "none",
      lockedOut: (names: string) =>
        `These will become unreachable for good: ${names}. Password logins are off on them, and the key exists only here.`,
      lockedOutAdvice:
        "Save the key to a file before removing, or the way back to such a server is your hosting provider's console and a reinstall.",
      agree: "I understand this cannot be undone",
      remove: "Remove everything",
      removing: "Removing…",
      done: "Removed. The application can be uninstalled — nothing of yours is left behind.",
      secretsLeft: (names: string) =>
        `The system store would not give up these secrets: ${names}. They will have to be cleared by hand.`,
      dirLeft: "The data directory could not be removed — something may be holding a file in it.",
    },
    update: {
      title: "Updates",
      installed: "Version installed",
      check: "Check for updates",
      checking: "Checking…",
      upToDate: "Nothing newer.",
      notConfigured:
        "This build carries no update settings, so there is nowhere to look. Builds from source are like this; released copies update.",
      unpackaged:
        "This is a build from the source tree, not an installed copy: there is nothing here to update.",
      available: (version: string) => `Version ${version} is out.`,
      published: "Published",
      notes: "What is in it",
      install: "Update",
      installing: "Installing…",
      warnWindows:
        "The installer stops the application the moment it starts. Starting it again is up to you.",
      warnPackage:
        "The system will ask for an administrator password: the package is installed as root. The application carries on running — the new version begins at the next start.",
      warnAppImage:
        "The application will rewrite its own file and carry on running as before. The new version begins at the next start.",
      // Neutral, because the particulars differ: being stopped, an administrator password,
      // a rewritten file — each is named by the warning right above the checkbox. One
      // wording covering all three would be true on exactly one platform.
      agree: "I understand a new version will be installed",
      doneRestartLater: "The update is installed. The new version begins at the next start.",
    },
    appearance: {
      title: "Appearance",
      theme: "Theme",
      language: "Language",

      workDir: "Working files",
      workDirMeans:
        "While a variant is being made it sits on a disk whole — one and a half to two gigabytes at a time. Beside the source film by default: the disk a film is on certainly fits a film. Change it if you keep a separate disk for scratch work.",
      workDirDefault: "Beside the source file",
      workDirPick: "Choose a folder",
      workDirReset: "Back to the default",
      workDirLeft:
        "The old folder still holds {files} file(s), {mb} MB in all. The application will not look there again — remove them yourself if they are not wanted.",

      mascot: "Mascot",
      mascotOn: "Show the mascot",
      mascotMeans:
        "It shows the same things the task list does: work, success, failure and a viewer in trouble. Turned off, it is not loaded at all rather than merely hidden.",
      animations: "Motion",
      animationsOn: "Smooth transitions",
      animationsMeans:
        "Transitions between sections and the mascot's movement. Turned off here or by the system's \u00abreduce motion\u00bb setting \u2014 what the system has turned off cannot be turned back on here.",

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
      rawHint:
        "The judgement above was made out of these figures. They are shown so it can be argued with: it is sometimes wrong.",
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
      logsRangesBad:
        "Whole files are served more often than ranges. Watching works, seeking does not.",
      logsTopPaths: "Asked for most often",
      logsTopAddresses: "Asked most often",
      logsFailures: "Failures",
      logsNoFailures: "No failures.",
      logsLong: "Long requests",
      logsLongNormal:
        "A long request is normally fine: it is a long range fetch. Only the ones that delivered next to nothing are marked.",
      logsCapped:
        "Not everything is shown: this stretch holds more lines than could be brought across. Ask for a shorter one.",

      stallsTitle: "Why the picture stops",
      stallsNoViewers: "Nobody was watching over this stretch.",
      stallsSetAside: "Not viewers",
      stallsOurOwnCheck: "the server's own address — these are our own checks",
      stallsTooLittle: (n: number) =>
        `segments taken: ${n} — a cache filling itself, or somebody who has just arrived`,
      stallsRatio: "Content received against real time",
      stallsLink: "The viewer's link",
      stallsInDownload: "inside the downloads",
      stallsInDownloadHint:
        "The second figure is always the higher: it is the speed inside the downloads, without the pauses between them. The viewer's link is the first.",
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
      bitrateHint:
        "The file is read here and no server is touched. It can be asked before an upload too — which is when it helps most.",
      bitrateAverage: "Average",
      bitrateMedian: "Median",
      bitratePeak1: "One-second peak",
      bitratePeak10: "Ten-second peak",
      bitrateAt: "at",
      bitrateWorst: "Where it is heaviest",
      bitratePeakOverAverage: (times: number) =>
        `The ten-second peak is ${times} times the average.`,
      bitrateAdvice:
        "It is this peak that hangs a player whose link is below it. A re-encode with the peaks capped is the cure.",
      bitrateEven:
        "The file is even: its peaks are close to its average, and there is nothing to re-encode.",
    },
    upgrade: {
      title: "Update the server side",
      fromTo: (from: number, to: number) =>
        `The server is at version ${from}; this application deploys ${to}.`,
      willChange: "What will change",
      nothingToDo: "Everything is already in place — nothing to change.",
      willKeep: "What will be copied aside first",
      keepsVideosAndCatalogue:
        "The videos and the catalogue are neither copied nor touched: they are your work, not our configuration. A rollback that restored the catalogue would undo everything uploaded since.",
      agreeAndUpgrade: "Agreed — update",
      rollBack: "Put it back as it was",
      cancel: "Cancel",
    },
    limits: {
      title: "Capping quality",
      explain:
        "A player takes the best it is shown and will not be talked out of it. The only way to bring a viewer down to a rung they can hold is to stop showing them the ones they cannot.",
      pickMedia: "Which medium",
      cap: "Cap, Mbit/s",
      willGet: "This viewer would be given:",
      apply: "Cap it",
      confirm: "Understood — cap it",
      cancel: "Cancel",
      noLadder: "This medium has no quality set — there is nothing to shorten",

      listTitle: "Limits in force",
      listEmpty: "Nothing is capped.",
      listFromServer:
        "Read from the server rather than from a note here: a note goes stale the hour somebody edits the server by hand.",
      columnWho: "Address",
      columnMedia: "Medium",
      columnCap: "Cap",
      columnSince: "Since",
      remove: "Lift",
      removing: "Lifting…",
    },

    viewers: {
    placesMissing: "The tables of places have not been downloaded, so no country or city is shown for a viewer.",
    placesStale: "The tables of places are from {month}; a newer set is out, so places may be off.",
    placesFetch: "Download",
    placesFetching: "Downloading…",
    placesFailed: "The download did not go through — try again later.",
      explain:
        "Who is pulling from your server right now. The list keeps itself up to date while this screen is open.",
      noServer: "Choose a server first — there is nobody to watch yet.",
      starting: "Starting to watch…",
      nobody: "Nobody is watching at the moment.",
      notKnown: "not determined",
      watchingUnknown: "what they are watching is not known yet",
      speedNotYet: "A speed appears once there is enough to work one out from.",
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
        slowLinkHint:
          "Less is arriving than the quality they are getting needs. The player will not step down by itself — the quality has to be capped by hand.",
        retransmits: "a lossy link",
        retransmitsHint:
          "A noticeable share of what is sent has to be sent again. Usually the viewer's connection rather than the server's.",
        stalls: "the pulling has stopped",
        stallsHint:
          "The connection is open but nothing is moving. If it lasts, the viewer's film has cut out.",
      },
      watchingNow: "watching now",
    },

    sections: {
      servers: "Servers",
      library: "Library",
      convert: "Preparation",
      upload: "Upload",
      ladder: "Quality",
      viewers: "Viewers",
      limits: "Limits",
      diagnostics: "Diagnostics",
      appearance: "Appearance",
      tasks: "Tasks",
    },

    sidebar: {
      step: "{n}. {name}",
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
      importFound: "Settings from the old way of working were found nearby",
      importExplain:
        "— the address, domain, user and key path can be filled in from it. The file is only read, never changed.",
      importNeedsPassphrase:
        " The key's passphrase will have to be entered: it is not in the file.",
      importApply: "Fill in",
      fieldName: "Name",
      fieldNamePlaceholder: "How to tell this server from the others",
      fieldHost: "Address",
      fieldHostPlaceholder: "IP address or name",
      fieldPort: "Port",
      fieldDomain: "Serving domain",
      fieldDomainHint:
        "Viewer links are handed out on it. You can paste straight from the address bar — the extra parts are removed for you.",
      fieldUser: "User",
      fieldAuth: "Sign-in",
      authKey: "By key",
      authPassword: "By password",
      pickKey: "Browse\u2026",
    fieldKeyPath: "Path to the private key",
      fieldPassphrase: "Key passphrase",
      fieldPassword: "Password",
      secretHint:
        "Kept in the system password store, not in the application's files. It is never handed back to the application.",
      optional: "Optional",
      fieldVideoDir: "Video directory on the server",
      fieldVideoDirPlaceholder: "leave empty for the default",
      fieldCdn: "CDN address",
      fieldCdnPlaceholder: "empty = links only through the server itself",
      checking: "Checking…",
      next: "Next",
      fingerprintLead:
        "The server introduced itself with this fingerprint. Compare it with the one your hosting provider's control panel shows, then confirm it.",
      fingerprintWhy:
        "Until it is confirmed the application will send the server neither password nor key. That way an impersonating server gets none of your credentials, even if it manages to answer at the right address.",
      abandon: "Give up",
      fingerprintOk: "The fingerprint is right",
      testAgain: "Check again",
      done: "Done",
      testRunning: "Checking the connection…",
      stepSkipped: "not checked: we stopped earlier",
    },

    upload: {
      heading: "Upload",
      noServers: "Add a server in the Servers section first — there is nowhere to upload to yet.",
      noActive: "Choose an active server in the Servers section.",
      notReady:
        "The fingerprint of the server “{name}” is not confirmed. Until it is, the application will not connect to it.",
      lead: "The file will go to the server “{name}”. Uploading happens in the background — this screen can be closed and the task followed in the Tasks section.",
      fieldFile: "File",
      pickFile: "Choose a file…",
      pickTitle: "Choose the prepared file",
      pickFilter: "Video",
      fieldName: "Name in service",
      nameHint: "Viewers see the file under this name, and the link is built from it.",
      fieldMedia: "Assign to a medium",
      mediaNone: "do not assign — it will land in “not recognised”",
      fieldLimit: "Cap the speed",
      limitNone: "no cap",
      limit10: "10 Mbit/s",
      limit25: "25 Mbit/s",
      limit50: "50 Mbit/s",
      limit100: "100 Mbit/s",
      limitHintLead: "Useful if you need to watch something while uploading:",
      limitHintUnlimited: "with no cap the upload takes the whole connection",
      limitHintCapped: "no faster than {bytes|bytes} per second",
      started: "The upload has begun.",
      startedHint:
        "Follow it in the Tasks section. If the application is closed, it continues from where it got to at the next start.",
      checking: "Checking…",
      start: "Start the upload",
    },

    validation: {
      ok: "The file plays all the way through — it can be uploaded.",
      failed:
        "The file failed the playback check. It must not be uploaded: it will fall apart for a viewer in the same place it fell apart here.",
      decoderSaid: "What the decoder said:",
      ignoredSummary: "Timestamp complaints: {n} — they do not affect playback",
    },

    preflight: {
      uploadAnyway: "Upload anyway",
      understood: "Understood",
    },

    convert: {
      heading: "Preparation",
      trackFallback: "Track {n}",
      mono: "mono",
      stereo: "stereo",
      channels: "{n} channels",
      trackDefault: " (main)",
      trackLine: "{base}, {channels}{main}",
      pickSourceTitle: "Choose the source video",
      pickSourceFilter: "Video",
      pickOutputTitle: "Where to put the prepared file",
      fieldSource: "Source",
      pickFile: "Choose a file…",
      sourceFacts: "{width}×{height}, {fps} fps, {duration}, {size}, {codec}",
      fieldTrack: "Audio track",
      noTracks: "The file has no audio track at all — check that this is the right file.",
      fieldOutput: "Where to put it",
      pick: "Choose…",
      lossless:
        "There will be no re-encoding — the file is carried across as it is, without loss and in minutes.",
      lossy:
        "The file has to be re-encoded. That is hours where carrying it across would take minutes.",
      videoLine: "Video:",
      audioLine: "Audio:",
      copyAsIs: "carry across as it is",
      reencodeBecause: "re-encode — {reason}",
      started: "Preparation has begun.",
      startedHint:
        "Follow it in the Tasks section. At the end the file is checked for playback: one that fails the check is not offered for upload.",
      computing: "Working it out…",
      start: "Prepare",
      nextTitle: "The file is ready. What next",
      nextHint:
        "The usual way is to cut it into qualities: the viewer's player then takes whichever one their connection carries. Sending it as one file is simpler, but then every viewer gets the same weight.",
      nextLadder: "Cut into qualities",
      nextUpload: "Send as one file",
      nextFailed: "The preparation failed, so there is nothing to go on with. The task says what happened.",
    },

    library: {
      heading: "Library",
      reading: "Reading the library…",
      noActiveServer:
        "No active server is chosen. The library lives on a server — one has to be added first.",
      goToServers: "Go to servers",
      newMedia: "New medium",
      serverLine: "Server:",
      empty: "Nothing on the server yet. Create a medium and upload files into it.",
      mediaFacts: "{n} {n|plural:file} · {bytes|bytes}",
      hasLadder: " · quality ladder",
      missingOnServer: " · {n} not found on the server",
      shortName: "Short name:",
      ladders: "Quality ladders: {list}",
      renameMedia: "Rename",
      deleteMedia: "Delete the medium",
      diskFree: "Free",
      diskOf: "of",
      diskVideos: "video takes up {bytes|bytes}",
      diskLabel: "Disk space used on the server",
      staleTitle: "The server is out of reach right now",
      staleHint:
        "This is the last the application managed to learn. The files on the server have not gone anywhere — it is the connection that is not answering. Actions that change the library cannot be carried out until it comes back.",
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
      faststartWarning:
        "The header is not at the start of the file — a viewer will only begin watching once they have downloaded the whole thing. This file is worth preparing again.",
      missingWarning:
        "The file is not on the server: it was deleted or renamed outside the application. The link to it does not work.",
      deleteFile: "Delete the file",
      unrecognizedTitle: "Not recognised",
      unrecognizedCount: "{n} {n|plural:file} · {bytes|bytes}",
      unrecognizedNote:
        "These files are on the server but belong to no medium. They take up room and are served over direct links. Assign them to a medium — or delete them.",
      assignTo: "Assign to a medium",
      assignChoose: "— choose —",
      createHeading: "New medium",
      fieldTitle: "Title",
      fieldSlugOptional: "Short name (optional)",
      fieldSlugPlaceholder: "made from the title",
      slugHint: "It goes into file names and links: Latin letters, digits, hyphens, underscores.",
      creating: "Creating…",
      create: "Create",
      renameHeading: "Rename “{title}”",
      titleHint: "Only you see it. It leaves files and links alone.",
      fieldSlug: "Short name",
      slugChangeWarning:
        "The files on the server will be renamed and every link handed out before will stop working. If you have already given them to viewers, you will have to give them out again.",
      renaming: "Renaming…",
      rename: "Rename",
      deleteHeading: "Delete “{what}”?",
      deleteLabel: "Delete {what}",
      deleteIrreversible: "There will be no undoing it.",
      deleteNo: "Do not delete",
      deleting: "Deleting…",
      deleteYes: "Delete",
    },

    servers: {
      heading: "Servers",
      reading: "Reading the server list…",
      add: "Add a server",
      empty:
        "No servers yet. Add the first one — the application will learn its fingerprint, ask you to confirm it, and check the connection step by step.",
      activeBadge: "active",
      makeActive: "Make active",
      domain: "Domain",
      videoDir: "Video directory",
      cdn: "CDN",
      fingerprintUnconfirmed:
        "The server's fingerprint is not confirmed — connecting is not possible. The application does not send credentials to a server it does not recognise.",
      testing: "Checking…",
      test: "Check the connection",
      confirmRemoval:
        "Delete this profile? The password or key for this server will be forgotten too.",
      removeYes: "Yes, delete",
      remove: "Delete",
      steps: {
        network: "The server is reachable over the network",
        login: "Signing in to the server",
        video_dir: "The video directory is reachable",
        domain: "Serving answers on the domain",
      },
      stepStatus: { ok: "passed", failed: "failed", skipped: "not checked" },
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
      counts: "Running: {running}. Waiting: {queued}.",
    heading: "Tasks",
      reading: "Reading the task list…",
      empty: "No tasks yet. They will appear when you start preparing or uploading video.",
      speed: "{mbit} Mbit/s",
      etaHours: "~{h} h {m} min left",
      etaMinutes: "~{m} min left",
      etaSoon: "less than a minute left",
      pause: "Pause",
      resume: "Resume",
      stop: "Cancel",
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
      queueExplain:
        "Tasks will run in this order. Reordering leaves a task that has already started alone — it would have to be interrupted, losing the work done.",
      moveUp: "Move up the queue",
      moveDown: "Move down the queue",
      closeLosing: "Closing the application now would lose some of the work",
      closeSafe: "The application can be closed: unfinished work will continue at the next start",
    },

    notifications: {
      completed: "Task finished",
      failed: "Task failed",
      lookInTasks: "The details are in the Tasks section.",
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
