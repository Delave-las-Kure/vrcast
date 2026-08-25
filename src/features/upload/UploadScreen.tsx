/**
 * T098 — экран заливки.
 *
 * Что здесь происходит: выбрать готовый файл, сказать, под каким именем он станет
 * виден зрителям и к какому медиа относится, при желании ограничить скорость —
 * и поставить задачу. Дальше всё видно в разделе задач: заливка идёт часами,
 * и держать ради неё открытым этот экран человек не обязан.
 *
 * Окно выбора файла — системное (плагин `dialog`). Своё, из веб-окна, не годится:
 * у выбранного там файла нет пути на диске, а заливке нужен именно путь.
 *
 * Проверки до старта в интерфейсе не дублируются. Их делает ядро — оно одно знает
 * состояние сервера, — а экран лишь показывает готовый ответ и спрашивает согласия
 * там, где оно уместно (см. `PreflightWarnings`).
 */

import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppError, LibraryView, UploadRequest } from "../../shared/contract";
import { formatBytes } from "../../shared/format";
import { ipc, toAppError } from "../../shared/ipc";
import { isReady, useActiveServer, useServers } from "../servers/store";
import { ErrorNotice } from "../shared/ErrorNotice";
import { PreflightWarnings, canConfirm } from "./PreflightWarnings";

/** Пределы скорости на выбор. Значения — байты в секунду, как их ждёт ядро. */
const ПРЕДЕЛЫ: Array<{ label: string; value: number | null }> = [
  { label: "не ограничивать", value: null },
  { label: "10 Мбит/с", value: 1_250_000 },
  { label: "25 Мбит/с", value: 3_125_000 },
  { label: "50 Мбит/с", value: 6_250_000 },
  { label: "100 Мбит/с", value: 12_500_000 },
];

/** Имя файла из полного пути — с любым разделителем. */
function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

export function UploadScreen() {
  const profiles = useServers((s) => s.profiles);
  const reloadServers = useServers((s) => s.reload);
  const active = useActiveServer();

  const [localPath, setLocalPath] = useState<string | null>(null);
  const [remoteName, setRemoteName] = useState("");
  const [mediaId, setMediaId] = useState<string>("");
  const [limitBps, setLimitBps] = useState<number | null>(null);

  const [library, setLibrary] = useState<LibraryView | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  /** Отказ до старта: его показывает `PreflightWarnings`, а не общий показ ошибок. */
  const [preflight, setPreflight] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [startedTask, setStartedTask] = useState<string | null>(null);

  useEffect(() => {
    void reloadServers();
  }, [reloadServers]);

  // Список медиа нужен, чтобы отнести файл сразу. Недоступный сервер — не повод
  // ломать экран: заливать всё равно нельзя, и об этом сказано отдельно.
  useEffect(() => {
    if (!active) {
      setLibrary(null);
      return;
    }
    let живо = true;
    void ipc
      .libraryList(active.id)
      .then((v) => {
        if (живо) setLibrary(v);
      })
      .catch(() => {
        if (живо) setLibrary(null);
      });
    return () => {
      живо = false;
    };
  }, [active]);

  const media = useMemo(() => library?.media ?? [], [library]);

  const выбрать = async () => {
    const выбранное = await open({
      multiple: false,
      directory: false,
      title: "Выберите готовый файл",
      filters: [{ name: "Видео", extensions: ["mp4", "mkv", "mov", "webm", "m4v"] }],
    });
    if (typeof выбранное !== "string") return;

    setLocalPath(выбранное);
    // Имя в раздаче подставляется из имени файла: чаще всего оно и нужно, а поле
    // остаётся открытым для правки.
    if (!remoteName) setRemoteName(basename(выбранное));
    setPreflight(null);
    setStartedTask(null);
  };

  const залить = async (confirmed: boolean) => {
    if (!active || !localPath) return;
    setBusy(true);
    setError(null);

    const request: UploadRequest = {
      server_id: active.id,
      local_path: localPath,
      remote_name: remoteName.trim(),
      media_id: mediaId === "" ? null : mediaId,
      limit_bps: limitBps,
      confirmed,
    };

    try {
      const taskId = await ipc.uploadStart(request);
      setStartedTask(taskId);
      setPreflight(null);
    } catch (e) {
      const err = toAppError(e);
      // Отказ, о котором можно спросить, и отказ, с которым спорить нечем,
      // показываются по-разному — но оба здесь, рядом с кнопкой, а не где-то ещё.
      if (canConfirm(err) || err.code === "REMOTE_DISK_FULL") setPreflight(err);
      else setError(err);
    } finally {
      setBusy(false);
    }
  };

  const готово =
    active !== null && isReady(active) && localPath !== null && remoteName.trim() !== "";

  if (profiles.length === 0) {
    return (
      <div className="panel">
        <h1>Заливка</h1>
        <p className="muted">
          Сначала заведите сервер в разделе «Серверы» — заливать пока некуда.
        </p>
      </div>
    );
  }

  return (
    <div className="panel">
      <h1>Заливка</h1>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {active === null ? (
        <p className="muted">Выберите активный сервер в разделе «Серверы».</p>
      ) : !isReady(active) ? (
        <p className="muted">
          У сервера «{active.name}» не подтверждён отпечаток. Пока это не сделано,
          приложение к нему не подключится.
        </p>
      ) : (
        <>
          <p className="muted">
            Файл уйдёт на сервер «{active.name}». Заливка идёт в фоне — этот экран
            можно закрыть, а следить за ней в разделе «Задачи».
          </p>

          <div className="form">
            <div className="form__row">
              <label htmlFor="upload-file">Файл</label>
              <div className="form__inline">
                <button id="upload-file" onClick={() => void выбрать()} disabled={busy}>
                  Выбрать файл…
                </button>
                {localPath && <span className="form__value">{localPath}</span>}
              </div>
            </div>

            <div className="form__row">
              <label htmlFor="upload-name">Имя в раздаче</label>
              <input
                id="upload-name"
                value={remoteName}
                onChange={(e) => {
                  setRemoteName(e.target.value);
                  setPreflight(null);
                }}
                placeholder="film_22.mp4"
              />
              <p className="form__hint">
                Под этим именем файл увидят зрители и по нему же строится ссылка.
              </p>
            </div>

            <div className="form__row">
              <label htmlFor="upload-media">Отнести к медиа</label>
              <select
                id="upload-media"
                value={mediaId}
                onChange={(e) => setMediaId(e.target.value)}
              >
                <option value="">не относить — попадёт в «не распознано»</option>
                {media.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.title}
                  </option>
                ))}
              </select>
            </div>

            <div className="form__row">
              <label htmlFor="upload-limit">Ограничить скорость</label>
              <select
                id="upload-limit"
                value={limitBps === null ? "" : String(limitBps)}
                onChange={(e) =>
                  setLimitBps(e.target.value === "" ? null : Number(e.target.value))
                }
              >
                {ПРЕДЕЛЫ.map((p) => (
                  <option key={p.label} value={p.value === null ? "" : String(p.value)}>
                    {p.label}
                  </option>
                ))}
              </select>
              <p className="form__hint">
                Пригодится, если во время заливки нужно ещё и смотреть:{" "}
                {limitBps === null
                  ? "без предела заливка займёт весь канал"
                  : `не быстрее ${formatBytes(limitBps)} в секунду`}
                .
              </p>
            </div>
          </div>

          {preflight && (
            <PreflightWarnings
              error={preflight}
              busy={busy}
              onConfirm={() => void залить(true)}
              onCancel={() => setPreflight(null)}
            />
          )}

          {startedTask && (
            <div className="notice notice--ok" role="status">
              <div className="notice__body">
                <strong className="notice__message">Заливка началась.</strong>
                <p className="notice__hint">
                  Следить за ней — в разделе «Задачи». Если закрыть приложение,
                  она продолжится с достигнутого места при следующем запуске.
                </p>
              </div>
            </div>
          )}

          <div className="form__actions">
            <button
              className="button--primary"
              disabled={!готово || busy}
              onClick={() => void залить(false)}
            >
              {busy ? "Проверяем…" : "Залить"}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
