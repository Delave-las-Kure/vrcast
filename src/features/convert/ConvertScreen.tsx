/**
 * T122 — the file preparation screen (FR-020, FR-021, FR-025, FR-026).
 *
 * Pick a source, see what is actually in it, choose the audio track, and start.
 *
 * The screen's real job is the preview. Copying a compatible file takes minutes;
 * re-encoding takes hours, and the difference is invisible from the outside —
 * both look like "prepare the file". Saying which one is about to happen, and
 * why, is the whole reason this is not just a button.
 */

import { useEffect, useMemo, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { AppError, ConvertPreview, ConvertStart, SourceFile } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT, type Catalogue, type Lang } from "../../shared/i18n";
import { formatBytes, formatDuration } from "../../shared/i18n/format";
import { fill, renderDetail } from "../../shared/i18n/render";

/**
 * Bitrate choices, in kilobits — the unit the core expects.
 *
 * The value is fixed and the label is a catalogue key: the numbers mean the same in
 * every language, the words around them do not.
 */
const BITRATES: Array<{ key: keyof Catalogue["ui"]["convert"]; value: number | null }> = [
  { key: "bitrateSource", value: null },
  { key: "bitrate9", value: 9_000 },
  { key: "bitrate14", value: 14_000 },
  { key: "bitrate22", value: 22_000 },
  { key: "bitrate35", value: 35_000 },
];

/** Name a track the way a person can choose between two of them. */
function trackLabel(
  track: SourceFile["audio_tracks"][number],
  t: Catalogue,
  lang: Lang,
): string {
  const named = [track.language, track.title].filter(Boolean).join(" — ");
  // Numbered from one: "track 0" reads like a bug report, not a choice.
  const base =
    named || fill(t.ui.convert.trackFallback, { n: track.index + 1 }, t, lang);
  const channels =
    track.channels === 1
      ? t.ui.convert.mono
      : track.channels === 2
        ? t.ui.convert.stereo
        : fill(t.ui.convert.channels, { n: track.channels }, t, lang);
  return fill(
    t.ui.convert.trackLine,
    { base, channels, main: track.is_default ? t.ui.convert.trackDefault : "" },
    t,
    lang,
  );
}

/** Suggest where to put the result, next to the source. */
function suggestOutput(sourcePath: string): string {
  const dot = sourcePath.lastIndexOf(".");
  const stem = dot > 0 ? sourcePath.slice(0, dot) : sourcePath;
  return `${stem}.ready.mp4`;
}

export function ConvertScreen() {
  const [sourcePath, setSourcePath] = useState<string | null>(null);
  const [source, setSource] = useState<SourceFile | null>(null);
  const [track, setTrack] = useState(0);
  const [targetKbps, setTargetKbps] = useState<number | null>(null);
  const [outPath, setOutPath] = useState("");

  const [preview, setPreview] = useState<ConvertPreview | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [startedTask, setStartedTask] = useState<string | null>(null);
  const t = useT();
  const { lang } = useLang();
  const c = t.ui.convert;

  const request: ConvertStart | null = useMemo(
    () =>
      sourcePath === null
        ? null
        : {
            path: sourcePath,
            audio_track: track,
            target_kbps: targetKbps,
            height: null,
            out_path: outPath,
            prefer_hardware: true,
          },
    [sourcePath, track, targetKbps, outPath],
  );

  // The preview is recomputed whenever the answer would change. It touches no
  // files and starts nothing, so there is no reason to make the person ask.
  useEffect(() => {
    if (!request || !source) {
      setPreview(null);
      return;
    }
    let live = true;
    void ipc
      .convertPreview(request)
      .then((p) => {
        if (live) {
          setPreview(p);
          setError(null);
        }
      })
      .catch((e) => {
        if (live) {
          setPreview(null);
          setError(toAppError(e));
        }
      });
    return () => {
      live = false;
    };
  }, [request, source]);

  const pickSource = async () => {
    const chosen = await open({
      multiple: false,
      directory: false,
      title: c.pickSourceTitle,
      filters: [
        {
          name: c.pickSourceFilter,
          extensions: ["mp4", "mkv", "mov", "webm", "m4v", "avi", "ts"],
        },
      ],
    });
    if (typeof chosen !== "string") return;

    setBusy(true);
    setStartedTask(null);
    try {
      const probed = await ipc.sourceProbe(chosen);
      setSourcePath(chosen);
      setSource(probed);
      setOutPath(suggestOutput(chosen));
      // The track marked as the main one, or the first — never a silent zero when
      // the file has none.
      setTrack(probed.audio_tracks.find((track) => track.is_default)?.index ?? 0);
      setError(null);
    } catch (e) {
      setSource(null);
      setSourcePath(null);
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const pickOutput = async () => {
    const chosen = await save({
      title: c.pickOutputTitle,
      defaultPath: outPath || undefined,
      filters: [{ name: "MP4", extensions: ["mp4"] }],
    });
    if (typeof chosen === "string") setOutPath(chosen);
  };

  const start = async () => {
    if (!request) return;
    setBusy(true);
    setError(null);
    try {
      setStartedTask(await ipc.convertStart(request));
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = request !== null && outPath.trim() !== "" && preview !== null;

  return (
    <div className="panel">
      <h1>{c.heading}</h1>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <div className="form">
        <div className="form__row">
          <label htmlFor="convert-file">{c.fieldSource}</label>
          <div className="form__inline">
            <button id="convert-file" onClick={() => void pickSource()} disabled={busy}>
              {c.pickFile}
            </button>
            {sourcePath && <span className="form__value">{sourcePath}</span>}
          </div>
        </div>

        {source && (
          <>
            <p className="form__hint">
              {fill(
                c.sourceFacts,
                {
                  width: source.width,
                  height: source.height,
                  fps: source.fps,
                  duration: formatDuration(source.duration_s),
                  size: formatBytes(source.size_bytes, lang),
                  codec: source.video_codec,
                },
                t,
                lang,
              )}
              {source.color_transfer ? `, ${source.color_transfer}` : ""}
            </p>

            <div className="form__row">
              <label htmlFor="convert-track">{c.fieldTrack}</label>
              {source.audio_tracks.length === 0 ? (
                <p className="form__hint">{c.noTracks}</p>
              ) : (
                <select
                  id="convert-track"
                  value={track}
                  onChange={(e) => setTrack(Number(e.target.value))}
                >
                  {source.audio_tracks.map((track) => (
                    <option key={track.index} value={track.index}>
                      {trackLabel(track, t, lang)}
                    </option>
                  ))}
                </select>
              )}
            </div>

            <div className="form__row">
              <label htmlFor="convert-bitrate">{c.fieldBitrate}</label>
              <select
                id="convert-bitrate"
                value={targetKbps === null ? "" : String(targetKbps)}
                onChange={(e) =>
                  setTargetKbps(e.target.value === "" ? null : Number(e.target.value))
                }
              >
                {BITRATES.map((b) => (
                  <option key={b.key} value={b.value === null ? "" : String(b.value)}>
                    {c[b.key] as string}
                  </option>
                ))}
              </select>
              <p className="form__hint">{c.bitrateHint}</p>
            </div>

            <div className="form__row">
              <label htmlFor="convert-out">{c.fieldOutput}</label>
              <div className="form__inline">
                <button
                  id="convert-out"
                  onClick={() => void pickOutput()}
                  disabled={busy}
                >
                  {c.pick}
                </button>
                {outPath && <span className="form__value">{outPath}</span>}
              </div>
            </div>
          </>
        )}
      </div>

      {preview && (
        <section className={`notice ${preview.lossless ? "notice--ok" : "notice--warning"}`} role="status">
          <div className="notice__body">
            <strong className="notice__message">
              {preview.lossless ? c.lossless : c.lossy}
            </strong>
            <ul className="notice__list">
              <li>
                {c.videoLine}{" "}
                {preview.plan.video.kind === "copy"
                  ? c.copyAsIs
                  : fill(
                      c.reencodeBecause,
                      { reason: renderDetail(preview.plan.video.reason, t, lang) },
                      t,
                      lang,
                    )}
              </li>
              <li>
                {c.audioLine}{" "}
                {preview.plan.audio.kind === "copy"
                  ? c.copyAsIs
                  : fill(
                      c.reencodeBecause,
                      { reason: renderDetail(preview.plan.audio.reason, t, lang) },
                      t,
                      lang,
                    )}
              </li>
            </ul>
            {preview.encoder_notice && (
              <p className="notice__hint">
                {renderDetail(preview.encoder_notice, t, lang)}
              </p>
            )}
          </div>
        </section>
      )}

      {startedTask && (
        <div className="notice notice--ok" role="status">
          <div className="notice__body">
            <strong className="notice__message">{c.started}</strong>
            <p className="notice__hint">{c.startedHint}</p>
          </div>
        </div>
      )}

      <div className="form__actions">
        <button
          className="button--primary"
          disabled={!ready || busy}
          onClick={() => void start()}
        >
          {busy ? c.computing : c.start}
        </button>
      </div>
    </div>
  );
}
