/**
 * T673 — what may be pressed on a video in which state, and how a few of its numbers read.
 *
 * The permissions mirror the core's own table (`domain::video::allowed`, T672). The core is
 * the authority and refuses with `VIDEO_NOT_NOW` whatever this gets wrong; these exist so a
 * button that would only be refused is not offered in the first place.
 */

import type { AudioTrack, SourceFacts, SourceFile, VideoView } from "../../shared/contract";
import type { Catalogue, Lang } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

/** The stages a person sees on the bar, in order. `planned` is the plan, not a stage. */
export const STAGES = [
  "measuring",
  "encoding",
  "uploading",
  "cutting",
  "verifying",
  "done",
] as const;

/** What the file dialog offers. */
export const VIDEO_EXTENSIONS = ["mp4", "mkv", "mov", "webm", "m4v", "avi", "ts"];

const BUSY = ["working", "paused", "cancelling", "done"];

export function canStart(v: VideoView): boolean {
  return v.state === "ready" || (v.state === "planning" && !v.start_requested);
}

export function canPause(v: VideoView): boolean {
  return v.state === "working";
}

export function canResume(v: VideoView): boolean {
  return v.state === "paused";
}

/** Planning without «Start» has nothing running to cancel — «Remove» is the way out. */
export function canCancel(v: VideoView): boolean {
  return (
    v.state === "working" ||
    v.state === "paused" ||
    v.state === "problem" ||
    (v.state === "planning" && v.start_requested)
  );
}

export function canRemove(v: VideoView): boolean {
  return v.state !== "working" && v.state !== "cancelling";
}

export function canSetAudio(v: VideoView): boolean {
  return (v.stage === "planned" || v.stage === "measuring") && !BUSY.includes(v.state);
}

export function canSetName(v: VideoView): boolean {
  return v.media_id === null && !BUSY.includes(v.state);
}

/** The editor needs rungs and the source's facts to check them against. */
export function canSetRungs(v: VideoView): boolean {
  return (
    ["planning", "ready", "problem", "cancelled"].includes(v.state) &&
    v.plan !== null &&
    v.plan.rungs.length > 0 &&
    v.source !== null
  );
}

/** The plan is what is shown until the video has started going. */
export function showsPlan(v: VideoView): boolean {
  return v.stage === "planned" && !["working", "paused", "cancelling"].includes(v.state);
}

/** The source as the rung checker wants it — the same facts the core plans with. */
export function factsOf(source: SourceFile): SourceFacts {
  return {
    width: source.width,
    height: source.height,
    fps: source.fps,
    bitrate_bps: source.bitrate_bps,
    heavier_codec: source.video_codec.toLowerCase() === "hevc",
    native_height: null,
  };
}

/** Megabits as a person says them: «8», «2,5». */
export function megabits(bps: number, lang: Lang): string {
  const tenths = Math.round(bps / 100_000) / 10;
  const text = Number.isInteger(tenths) ? String(tenths) : tenths.toFixed(1);
  return lang === "ru" ? text.replace(".", ",") : text;
}

/** Name a track so two of them can be told apart. Numbered from one. */
export function trackLabel(track: AudioTrack, t: Catalogue, lang: Lang): string {
  const w = t.ui.video;
  const named = [track.language, track.title].filter(Boolean).join(" — ");
  const base = named || fill(w.trackFallback, { n: track.index + 1 }, t, lang);
  const channels =
    track.channels === 1
      ? w.mono
      : track.channels === 2
        ? w.stereo
        : fill(w.channels, { n: track.channels }, t, lang);
  return fill(
    w.trackLine,
    { base, channels, main: track.is_default ? w.trackDefault : "" },
    t,
    lang,
  );
}

/**
 * Put a newer view of a video into the list. An answer older than what is already shown is
 * dropped: an action's reply and the event about the same change can arrive in either order.
 */
export function upsert(list: VideoView[], v: VideoView): VideoView[] {
  const at = list.findIndex((x) => x.id === v.id);
  if (at < 0) return [...list, v];
  if (list[at].updated_at > v.updated_at) return list;
  const next = list.slice();
  next[at] = v;
  return next;
}

/** The list `videoList` gave, merged with whatever events got here first. */
export function mergeListed(current: VideoView[], listed: VideoView[]): VideoView[] {
  const known = new Map(current.map((v) => [v.id, v]));
  const out = listed.map((v) => {
    const seen = known.get(v.id);
    return seen && seen.updated_at > v.updated_at ? seen : v;
  });
  const listedIds = new Set(listed.map((v) => v.id));
  return [...out, ...current.filter((v) => !listedIds.has(v.id))];
}
