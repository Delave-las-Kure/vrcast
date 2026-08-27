/**
 * T057 — the "not recognised" group (FR-015).
 *
 * Files that are not in the catalogue. An ordinary sight on a server that was filled
 * by scripts or by hand. Hiding them is out of the question: a file invisible in the
 * application still takes up disk space and is still served over a direct link — while
 * the user believes the library is complete and cannot see where the room went.
 *
 * A file can be assigned to a medium from here. The application never does it by
 * itself, even where the name makes it obvious: a guessed connection, recorded without
 * asking, later diverges from what the person meant, and untangling that is harder
 * than pointing at it once.
 */

import { useState } from "react";
import type { FileView, MediaView } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { FileRow } from "./FileRow";

export function UnrecognizedGroup({
  files,
  media,
  onAssign,
  onDelete,
  disabled,
}: {
  files: FileView[];
  media: MediaView[];
  onAssign: (path: string, mediaId: string) => void;
  onDelete: (path: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const t = useT();
  const { lang } = useLang();
  const total = files.reduce((sum, f) => sum + f.size_bytes, 0);

  if (files.length === 0) return null;

  return (
    <section className="media media--unrecognized">
      <button className="media__head" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <span className="media__title">{t.ui.library.unrecognizedTitle}</span>
        <span className="media__facts">
          {fill(t.ui.library.unrecognizedCount, { n: files.length, bytes: total }, t, lang)}
        </span>
      </button>

      {open && (
        <>
          <p className="muted media__note">{t.ui.library.unrecognizedNote}</p>
          <ul className="file-list">
            {files.map((f) => (
              <div key={f.path} className="unrecognized__item">
                <FileRow file={f} onDelete={disabled ? undefined : onDelete} />
                {media.length > 0 && !disabled && (
                  <label className="unrecognized__assign">
                    <span>{t.ui.library.assignTo}</span>
                    <select
                      defaultValue=""
                      onChange={(e) => {
                        if (e.target.value) onAssign(f.path, e.target.value);
                      }}
                    >
                      <option value="">{t.ui.library.assignChoose}</option>
                      {media.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.title}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
              </div>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
