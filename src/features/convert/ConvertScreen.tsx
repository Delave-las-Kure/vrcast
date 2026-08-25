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
import { formatBytes, formatDuration } from "../../shared/format";
import { ipc, toAppError } from "../../shared/ipc";
import { ErrorNotice } from "../shared/ErrorNotice";

/** Bitrate choices, in kilobits — the unit the core expects. */
const BITRATES: Array<{ label: string; value: number | null }> = [
  { label: "как в источнике", value: null },
  { label: "9 Мбит/с — надёжно под слабый канал", value: 9_000 },
  { label: "14 Мбит/с", value: 14_000 },
  { label: "22 Мбит/с — хорошо для 1080p", value: 22_000 },
  { label: "35 Мбит/с", value: 35_000 },
];

/** Name a track the way a person can choose between two of them. */
function trackLabel(t: SourceFile["audio_tracks"][number]): string {
  const named = [t.language, t.title].filter(Boolean).join(" — ");
  // Numbered from one: "track 0" reads like a bug report, not a choice.
  const base = named || `Дорожка ${t.index + 1}`;
  const channels =
    t.channels === 1 ? "моно" : t.channels === 2 ? "стерео" : `${t.channels} каналов`;
  return `${base}, ${channels}${t.is_default ? " (основная)" : ""}`;
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
      title: "Выберите исходное видео",
      filters: [{ name: "Видео", extensions: ["mp4", "mkv", "mov", "webm", "m4v", "avi", "ts"] }],
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
      setTrack(probed.audio_tracks.find((t) => t.is_default)?.index ?? 0);
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
      title: "Куда положить подготовленный файл",
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
      <h1>Подготовка</h1>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <div className="form">
        <div className="form__row">
          <label htmlFor="convert-file">Исходник</label>
          <div className="form__inline">
            <button id="convert-file" onClick={() => void pickSource()} disabled={busy}>
              Выбрать файл…
            </button>
            {sourcePath && <span className="form__value">{sourcePath}</span>}
          </div>
        </div>

        {source && (
          <>
            <p className="form__hint">
              {source.width}×{source.height}, {source.fps} кадр/с,{" "}
              {formatDuration(source.duration_s)}, {formatBytes(source.size_bytes)},{" "}
              {source.video_codec}
              {source.color_transfer ? `, ${source.color_transfer}` : ""}
            </p>

            <div className="form__row">
              <label htmlFor="convert-track">Звуковая дорожка</label>
              {source.audio_tracks.length === 0 ? (
                <p className="form__hint">
                  В файле нет ни одной звуковой дорожки — проверьте, тот ли это файл.
                </p>
              ) : (
                <select
                  id="convert-track"
                  value={track}
                  onChange={(e) => setTrack(Number(e.target.value))}
                >
                  {source.audio_tracks.map((t) => (
                    <option key={t.index} value={t.index}>
                      {trackLabel(t)}
                    </option>
                  ))}
                </select>
              )}
            </div>

            <div className="form__row">
              <label htmlFor="convert-bitrate">Целевой битрейт</label>
              <select
                id="convert-bitrate"
                value={targetKbps === null ? "" : String(targetKbps)}
                onChange={(e) =>
                  setTargetKbps(e.target.value === "" ? null : Number(e.target.value))
                }
              >
                {BITRATES.map((b) => (
                  <option key={b.label} value={b.value === null ? "" : String(b.value)}>
                    {b.label}
                  </option>
                ))}
              </select>
              <p className="form__hint">
                Заданный битрейт означает пересжатие, даже если файл и так подходит:
                иначе требование осталось бы невыполненным.
              </p>
            </div>

            <div className="form__row">
              <label htmlFor="convert-out">Куда положить</label>
              <div className="form__inline">
                <button id="convert-out" onClick={() => void pickOutput()} disabled={busy}>
                  Выбрать…
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
              {preview.lossless
                ? "Пересжатия не будет — файл перенесётся как есть, без потерь и за минуты."
                : "Файл придётся пересжать. Это часы работы там, где перенос занял бы минуты."}
            </strong>
            <ul className="notice__list">
              <li>
                Видео:{" "}
                {preview.plan.video.kind === "copy"
                  ? "перенести как есть"
                  : `пересжать — ${preview.plan.video.reason}`}
              </li>
              <li>
                Звук:{" "}
                {preview.plan.audio.kind === "copy"
                  ? "перенести как есть"
                  : `пересжать — ${preview.plan.audio.reason}`}
              </li>
            </ul>
            {preview.encoder_notice && (
              <p className="notice__hint">{preview.encoder_notice}</p>
            )}
          </div>
        </section>
      )}

      {startedTask && (
        <div className="notice notice--ok" role="status">
          <div className="notice__body">
            <strong className="notice__message">Подготовка началась.</strong>
            <p className="notice__hint">
              Следить за ней — в разделе «Задачи». В конце файл будет проверен
              на воспроизведение: не прошедший проверку к заливке не предлагается.
            </p>
          </div>
        </div>
      )}

      <div className="form__actions">
        <button
          className="button--primary"
          disabled={!ready || busy}
          onClick={() => void start()}
        >
          {busy ? "Считаем…" : "Подготовить"}
        </button>
      </div>
    </div>
  );
}
