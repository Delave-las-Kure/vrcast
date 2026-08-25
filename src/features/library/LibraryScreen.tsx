/**
 * T054 — библиотека: медиа и их файлы.
 *
 * Библиотека медиа-центрична: человек думает о произведении, а файлы — его варианты
 * по качеству. Поэтому список — это список медиа, а файлы раскрываются внутри.
 *
 * Показ идёт из кеша мгновенно, обновление приходит следом (FR-080): ждать ответа
 * сервера, чтобы показать список, который и так известен, незачем — по медленному
 * каналу это секунды пустого экрана.
 *
 * Удаление устроено в два вызова, и это не лишний оборот. Первый вызов — без
 * подтверждения — ядро отклоняет и в отказе называет последствия: сколько файлов,
 * сколько места, идёт ли прямо сейчас раздача. Их и показывает диалог. Спрашивать
 * «вы уверены?», не назвав ничего, — значит получить «да», не сообщив ничего.
 */

import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { AppError, LibraryView, MediaView } from "../../shared/contract";
import { countOf, formatBytes, usedFraction } from "../../shared/format";
import { ipc, onLibraryChanged, toAppError } from "../../shared/ipc";
import { useActiveServer, useServers } from "../servers/store";
import { ErrorNotice } from "../shared/ErrorNotice";
import { FileRow } from "./FileRow";
import { StaleBanner } from "./StaleBanner";
import { UnrecognizedGroup } from "./UnrecognizedGroup";
import {
  ConfirmDeleteDialog,
  CreateMediaDialog,
  RenameMediaDialog,
} from "./dialogs/MediaDialogs";

/** Что сейчас открыто поверх списка. */
type Dialog =
  | { kind: "create" }
  | { kind: "rename"; media: MediaView }
  | { kind: "delete"; media: MediaView; consequences: string }
  | { kind: "delete-file"; path: string; consequences: string }
  | null;

export function LibraryScreen() {
  const active = useActiveServer();
  const reloadServers = useServers((s) => s.reload);
  const serversLoading = useServers((s) => s.loading);

  const [view, setView] = useState<LibraryView | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [dialogError, setDialogError] = useState<AppError | null>(null);

  useEffect(() => {
    void reloadServers();
  }, [reloadServers]);

  const load = useCallback(
    async (refresh: boolean) => {
      if (!active) {
        setView(null);
        setLoading(false);
        return;
      }
      try {
        setView(await ipc.libraryList(active.id, refresh));
        setError(null);
      } catch (e) {
        setError(toAppError(e));
      } finally {
        setLoading(false);
      }
    },
    [active],
  );

  // Первый показ — из кеша, мгновенно.
  useEffect(() => {
    setLoading(true);
    void load(false);
  }, [load]);

  // Обновление из ядра: и то, что пришло фоном, и то, что вызвали мы сами.
  useEffect(() => {
    let cancelled = false;
    const unlisten: Array<() => void> = [];
    const keep = (fn: () => void) => {
      if (cancelled) fn();
      else unlisten.push(fn);
    };

    void onLibraryChanged(() => void load(false)).then(keep);

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [load]);

  /** Выполнить изменение и перечитать библиотеку. */
  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setDialogError(null);
    try {
      await fn();
      setDialog(null);
      await load(true);
    } catch (e) {
      setDialogError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  /**
   * Спросить у ядра последствия удаления и показать их.
   *
   * Отказ с кодом CONFIRMATION_REQUIRED — не ошибка, а ожидаемый ответ: это и есть
   * запрос подтверждения вместе с числами, которых интерфейс сам не знает.
   */
  const askBeforeDelete = async (media: MediaView) => {
    if (!active) return;
    try {
      await ipc.mediaDelete(active.id, media.id, false);
      // Ядро согласилось без подтверждения — такого быть не должно, но если
      // случилось, список надо привести в соответствие.
      await load(true);
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "CONFIRMATION_REQUIRED") {
        setDialog({ kind: "delete", media, consequences: err.message });
      } else {
        setError(err);
      }
    }
  };

  const askBeforeDeleteFile = async (path: string) => {
    if (!active) return;
    try {
      await ipc.fileDelete(active.id, path, false);
      await load(true);
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "CONFIRMATION_REQUIRED") {
        setDialog({ kind: "delete-file", path, consequences: err.message });
      } else {
        setError(err);
      }
    }
  };

  if (serversLoading || loading) {
    return <div className="panel">Читаем библиотеку…</div>;
  }

  if (!active) {
    return (
      <div className="panel">
        <h1>Библиотека</h1>
        <p className="muted">
          Активный сервер не выбран. Библиотека живёт на сервере — сначала нужно его
          добавить.
        </p>
        <Link className="button-link" to="/servers">
          Перейти к серверам
        </Link>
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel__head">
        <h1>Библиотека</h1>
        <div className="panel__head-actions">
          <button onClick={() => void load(true)} disabled={busy}>
            Обновить
          </button>
          <button onClick={() => setDialog({ kind: "create" })} disabled={busy}>
            Новое медиа
          </button>
        </div>
      </div>

      <p className="muted library__server">
        Сервер: <strong>{active.name}</strong> · {active.domain}
      </p>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      {view?.stale && <StaleBanner onRetry={() => void load(true)} />}
      {view?.disk && <DiskBar disk={view.disk} />}

      {dialog?.kind === "create" && (
        <CreateMediaDialog
          busy={busy}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onCreate={(title, slug) =>
            void act(() => ipc.mediaCreate(active.id, title, slug))
          }
        />
      )}
      {dialog?.kind === "rename" && (
        <RenameMediaDialog
          media={dialog.media}
          busy={busy}
          error={dialogError}
          onCancel={() => setDialog(null)}
          onRename={(title, slug) =>
            void act(() => ipc.mediaRename(active.id, dialog.media.id, title, slug))
          }
        />
      )}
      {dialog?.kind === "delete" && (
        <ConfirmDeleteDialog
          what={dialog.media.title}
          consequences={dialog.consequences}
          busy={busy}
          onCancel={() => setDialog(null)}
          onConfirm={() =>
            void act(() => ipc.mediaDelete(active.id, dialog.media.id, true))
          }
        />
      )}
      {dialog?.kind === "delete-file" && (
        <ConfirmDeleteDialog
          what={dialog.path}
          consequences={dialog.consequences}
          busy={busy}
          onCancel={() => setDialog(null)}
          onConfirm={() => void act(() => ipc.fileDelete(active.id, dialog.path, true))}
        />
      )}

      {view && view.media.length === 0 && view.unrecognized.length === 0 ? (
        <p className="muted">
          На сервере пока пусто. Создайте медиа — и заливайте в него файлы.
        </p>
      ) : (
        <div className="media-list">
          {view?.media.map((m) => (
            <MediaCard
              key={m.id}
              media={m}
              disabled={busy || view.stale}
              onRename={() => setDialog({ kind: "rename", media: m })}
              onDelete={() => void askBeforeDelete(m)}
              onDeleteFile={(path) => void askBeforeDeleteFile(path)}
            />
          ))}
          {view && (
            <UnrecognizedGroup
              files={view.unrecognized}
              media={view.media}
              disabled={busy || view.stale}
              onAssign={(path, mediaId) =>
                void act(() => ipc.fileMove(active.id, path, mediaId, true))
              }
              onDelete={(path) => void askBeforeDeleteFile(path)}
            />
          )}
        </div>
      )}
    </div>
  );
}

function MediaCard({
  media,
  disabled,
  onRename,
  onDelete,
  onDeleteFile,
}: {
  media: MediaView;
  disabled?: boolean;
  onRename: () => void;
  onDelete: () => void;
  onDeleteFile: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const missing = media.files.filter((f) => !f.exists_on_server).length;

  return (
    <section className="media">
      <button className="media__head" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <span className="media__title">{media.title}</span>
        <span className="media__facts">
          {countOf(media.files.length, "файл", "файла", "файлов")} ·{" "}
          {formatBytes(media.total_bytes)}
          {media.ladders.length > 0 && " · набор качеств"}
          {missing > 0 && (
            <em className="media__missing"> · {missing} не найдено на сервере</em>
          )}
        </span>
      </button>

      {open && (
        <>
          <p className="muted media__note">
            Короткое имя: <code>{media.slug}</code>
          </p>

          <ul className="file-list">
            {media.files.map((f) => (
              <FileRow
                key={f.path}
                file={f}
                onDelete={disabled ? undefined : onDeleteFile}
              />
            ))}
          </ul>

          {media.ladders.length > 0 && (
            <p className="muted media__note">
              Наборы качеств: {media.ladders.join(", ")}
            </p>
          )}

          <div className="media__actions">
            <button onClick={onRename} disabled={disabled}>
              Переименовать
            </button>
            <button className="button--danger" onClick={onDelete} disabled={disabled}>
              Удалить медиа
            </button>
          </div>
        </>
      )}
    </section>
  );
}

/** Место на диске сервера (FR-017). */
function DiskBar({ disk }: { disk: NonNullable<LibraryView["disk"]> }) {
  const used = usedFraction(disk.total_bytes, disk.free_bytes);
  return (
    <div className="disk">
      <div className="disk__facts">
        <span>
          Свободно <strong>{formatBytes(disk.free_bytes)}</strong> из{" "}
          {formatBytes(disk.total_bytes)}
        </span>
        <span className="muted">
          видео занимают {formatBytes(disk.used_by_videos_bytes)}
        </span>
      </div>
      <div
        className="progress"
        role="progressbar"
        aria-valuenow={Math.round(used * 100)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label="Занято места на диске сервера"
      >
        <div
          className={`progress__fill ${used > 0.9 ? "progress__fill--alarm" : ""}`}
          style={{ width: `${used * 100}%` }}
        />
      </div>
    </div>
  );
}
