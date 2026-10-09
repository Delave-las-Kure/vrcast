/**
 * T696 (owner's decision В3) — which subtitles are burned into the picture.
 *
 * One line in the plan: «Субтитры» and a list — «без субтитров» (the default) or a track by
 * its language, title and «signs only» mark. Burned into every rung, so it is chosen at the
 * same moments as the sound: before anything is encoded. Shown only when the film has a
 * track that can be burned at all.
 */

import type { SubtitleTrack, VideoView } from "../../shared/contract";
import { ipc } from "../../shared/ipc";
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { canSetAudio } from "./rules";

/** The line a person picks a subtitle track by. */
export function subtitleLabel(track: SubtitleTrack, t: Catalogue, lang: Lang): string {
  const w = t.ui.video;
  const named = [track.language, track.title].filter(Boolean).join(" — ");
  const base = named || fill(w.trackFallback, { n: track.index + 1 }, t, lang);
  return fill(w.subtitlesLine, { base, forced: track.forced ? w.subtitlesForced : "" }, t, lang);
}

export function SubtitlePick({
  video,
  busy,
  run,
}: {
  video: VideoView;
  busy: boolean;
  /** The card's own runner: shows the refusal, takes the new view. */
  run: (act: () => Promise<VideoView>) => void;
}) {
  const t = useT();
  const { lang } = useLang();
  const w = t.ui.video;
  const tracks = (video.source?.subtitle_tracks ?? []).filter((s) => s.kind !== "other");
  if (tracks.length === 0) return null;

  return (
    <label className="video__audio video__subtitles">
      <span>{w.subtitles}</span>
      <select
        value={video.subtitle_track === null ? "" : String(video.subtitle_track)}
        disabled={busy || !canSetAudio(video)}
        onChange={(e) => {
          const value = e.target.value;
          run(() => ipc.videoSetSubtitles(video.id, value === "" ? null : Number(value)));
        }}
      >
        <option value="">{w.subtitlesNone}</option>
        {tracks.map((track) => (
          <option key={track.index} value={track.index}>
            {subtitleLabel(track, t, lang)}
          </option>
        ))}
      </select>
    </label>
  );
}
