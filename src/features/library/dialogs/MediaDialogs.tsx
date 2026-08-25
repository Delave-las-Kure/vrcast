/**
 * T055 — диалоги изменения библиотеки.
 *
 * Общее правило всех трёх: **разрушительное действие подтверждается тем, что
 * названо**. «Вы уверены?» без чисел — не подтверждение, а формальность: человек
 * жмёт «да», не узнав ничего нового.
 *
 * Поэтому удаление не спрашивает вслепую. Ядро отказывает первым вызовом и в
 * отказе называет, сколько файлов исчезнет и сколько места освободится; диалог
 * показывает именно эту формулировку, а не сочиняет свою (FR-014, FR-105).
 */

import { useState } from "react";
import type { AppError, MediaView } from "../../../shared/contract";

/** Создание медиа. Короткое имя можно не задавать — ядро составит из названия. */
export function CreateMediaDialog({
  onCreate,
  onCancel,
  busy,
  error,
}: {
  onCreate: (title: string, slug: string | null) => void;
  onCancel: () => void;
  busy?: boolean;
  error?: AppError | null;
}) {
  const [title, setTitle] = useState("");
  const [slug, setSlug] = useState("");

  return (
    <form
      className="dialog"
      onSubmit={(e) => {
        e.preventDefault();
        onCreate(title.trim(), slug.trim() || null);
      }}
    >
      <h3>Новое медиа</h3>
      {error && <p className="dialog__error">{error.message}</p>}

      <label>
        <span>Название</span>
        <input value={title} onChange={(e) => setTitle(e.target.value)} required autoFocus />
      </label>
      {/* Пояснение вне метки: иначе оно становится частью названия поля. */}
      <div className="field">
        <label>
          <span>Короткое имя (необязательно)</span>
          <input
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
            placeholder="составится из названия"
          />
        </label>
        <small className="muted">
          Попадает в имена файлов и в ссылки: латинские буквы, цифры, дефис,
          подчёркивание.
        </small>
      </div>

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy}>
          Отмена
        </button>
        <button type="submit" disabled={busy || !title.trim()}>
          {busy ? "Создаём…" : "Создать"}
        </button>
      </div>
    </form>
  );
}

/**
 * Переименование.
 *
 * Смена короткого имени переименовывает файлы на сервере и **делает прежние ссылки
 * нерабочими**. Предупреждение появляется ровно тогда, когда имя действительно
 * меняют, — постоянная надпись быстро перестаёт читаться.
 */
export function RenameMediaDialog({
  media,
  onRename,
  onCancel,
  busy,
  error,
}: {
  media: MediaView;
  onRename: (title: string | null, slug: string | null) => void;
  onCancel: () => void;
  busy?: boolean;
  error?: AppError | null;
}) {
  const [title, setTitle] = useState(media.title);
  const [slug, setSlug] = useState(media.slug);

  const slugChanged = slug.trim() !== media.slug;
  const titleChanged = title.trim() !== media.title;
  const nothingChanged = !slugChanged && !titleChanged;

  return (
    <form
      className="dialog"
      onSubmit={(e) => {
        e.preventDefault();
        onRename(titleChanged ? title.trim() : null, slugChanged ? slug.trim() : null);
      }}
    >
      <h3>Переименовать «{media.title}»</h3>
      {error && <p className="dialog__error">{error.message}</p>}

      <div className="field">
        <label>
          <span>Название</span>
          <input value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
        </label>
        <small className="muted">Видно только вам. Файлы и ссылки не трогает.</small>
      </div>

      <label>
        <span>Короткое имя</span>
        <input value={slug} onChange={(e) => setSlug(e.target.value)} />
      </label>

      {slugChanged && (
        <p className="dialog__warning" role="status">
          Файлы на сервере будут переименованы, и все выданные раньше ссылки перестанут
          работать. Если вы уже раздали их зрителям, придётся раздать заново.
        </p>
      )}

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy}>
          Отмена
        </button>
        <button type="submit" disabled={busy || nothingChanged}>
          {busy ? "Переименовываем…" : "Переименовать"}
        </button>
      </div>
    </form>
  );
}

/**
 * Подтверждение удаления.
 *
 * `consequences` — формулировка от ядра: там названы число файлов, объём и, если
 * сервер прямо сейчас что-то отдаёт, число открытых соединений (FR-019a). Интерфейс
 * её показывает, а не переписывает: иначе формулировки разойдутся между экранами.
 */
export function ConfirmDeleteDialog({
  what,
  consequences,
  onConfirm,
  onCancel,
  busy,
}: {
  what: string;
  consequences: string;
  onConfirm: () => void;
  onCancel: () => void;
  busy?: boolean;
}) {
  return (
    <div className="dialog" role="alertdialog" aria-label={`Удалить ${what}`}>
      <h3>Удалить «{what}»?</h3>
      <p className="dialog__warning">{consequences}</p>
      <p className="muted">Отменить это будет нельзя.</p>

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy} autoFocus>
          Не удалять
        </button>
        <button
          type="button"
          className="button--danger"
          onClick={onConfirm}
          disabled={busy}
        >
          {busy ? "Удаляем…" : "Удалить"}
        </button>
      </div>
    </div>
  );
}
