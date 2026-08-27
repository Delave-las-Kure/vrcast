/**
 * T055 — the dialogs that change the library.
 *
 * One rule runs through all three: **a destructive action is confirmed by what it is
 * named to be**. "Are you sure?" without numbers is not a confirmation but a
 * formality — a person clicks yes having learnt nothing new.
 *
 * So deletion does not ask blindly. The core refuses the first call and, in refusing,
 * names how many files will vanish and how much room will be freed; the dialog shows
 * that wording, worded from the shared catalogue rather than invented here (FR-014,
 * FR-105).
 */

import { useState } from "react";
import type { AppError, MediaView } from "../../../shared/contract";
import { useLang, useT } from "../../../shared/i18n";
import { fill, renderError } from "../../../shared/i18n/render";

/** Creating a medium. The short name may be left out — the core makes one from the title. */
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
  const t = useT();
  const { lang } = useLang();
  const l = t.ui.library;

  return (
    <form
      className="dialog"
      onSubmit={(e) => {
        e.preventDefault();
        onCreate(title.trim(), slug.trim() || null);
      }}
    >
      <h3>{l.createHeading}</h3>
      {error && <p className="dialog__error">{renderError(error, t, lang).message}</p>}

      <label>
        <span>{l.fieldTitle}</span>
        <input value={title} onChange={(e) => setTitle(e.target.value)} required autoFocus />
      </label>
      {/* The explanation sits outside the label: inside, it becomes part of the
          field's name and is read aloud with it. */}
      <div className="field">
        <label>
          <span>{l.fieldSlugOptional}</span>
          <input
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
            placeholder={l.fieldSlugPlaceholder}
          />
        </label>
        <small className="muted">{l.slugHint}</small>
      </div>

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy}>
          {t.ui.common.cancel}
        </button>
        <button type="submit" disabled={busy || !title.trim()}>
          {busy ? l.creating : l.create}
        </button>
      </div>
    </form>
  );
}

/**
 * Renaming.
 *
 * Changing the short name renames the files on the server and **breaks every link
 * handed out before**. The warning appears exactly when the name is actually being
 * changed — a permanent notice soon stops being read.
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
  const t = useT();
  const { lang } = useLang();
  const l = t.ui.library;

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
      <h3>{fill(l.renameHeading, { title: media.title }, t, lang)}</h3>
      {error && <p className="dialog__error">{renderError(error, t, lang).message}</p>}

      <div className="field">
        <label>
          <span>{l.fieldTitle}</span>
          <input value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
        </label>
        <small className="muted">{l.titleHint}</small>
      </div>

      <label>
        <span>{l.fieldSlug}</span>
        <input value={slug} onChange={(e) => setSlug(e.target.value)} />
      </label>

      {slugChanged && (
        <p className="dialog__warning" role="status">
          {l.slugChangeWarning}
        </p>
      )}

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy}>
          {t.ui.common.cancel}
        </button>
        <button type="submit" disabled={busy || nothingChanged}>
          {busy ? l.renaming : l.rename}
        </button>
      </div>
    </form>
  );
}

/**
 * Confirming a deletion.
 *
 * `consequences` is the core's own account: it names the number of files, the size
 * and, if the server is serving something right now, the number of open connections
 * (FR-019a). The dialog shows it rather than rewriting it — otherwise the wordings
 * would drift apart between screens.
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
  const t = useT();
  const { lang } = useLang();
  const l = t.ui.library;

  return (
    <div className="dialog" role="alertdialog" aria-label={fill(l.deleteLabel, { what }, t, lang)}>
      <h3>{fill(l.deleteHeading, { what }, t, lang)}</h3>
      <p className="dialog__warning">{consequences}</p>
      <p className="muted">{l.deleteIrreversible}</p>

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy} autoFocus>
          {l.deleteNo}
        </button>
        <button type="button" className="button--danger" onClick={onConfirm} disabled={busy}>
          {busy ? l.deleting : l.deleteYes}
        </button>
      </div>
    </div>
  );
}
