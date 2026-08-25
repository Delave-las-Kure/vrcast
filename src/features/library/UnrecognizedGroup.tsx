/**
 * T057 — группа «не распознано» (FR-015).
 *
 * Файлы, которых нет в описи. Обычное дело на сервере, куда заливали скриптами
 * или руками. Прятать их нельзя ни в коем случае: файл, которого не видно
 * в приложении, всё равно занимает место на диске и всё равно отдаётся по прямой
 * ссылке — а пользователь считает библиотеку полной и не понимает, куда ушло место.
 *
 * Отсюда файл можно отнести к медиа. Приложение не делает этого само даже там, где
 * имя очевидно намекает: угаданная связь, записанная без спроса, потом расходится
 * с тем, что человек имел в виду, и разбираться в этом тяжелее, чем указать сразу.
 */

import { useState } from "react";
import type { FileView, MediaView } from "../../shared/contract";
import { countOf, formatBytes } from "../../shared/format";
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
  const total = files.reduce((sum, f) => sum + f.size_bytes, 0);

  if (files.length === 0) return null;

  return (
    <section className="media media--unrecognized">
      <button
        className="media__head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="media__title">Не распознано</span>
        <span className="media__facts">
          {countOf(files.length, "файл", "файла", "файлов")} · {formatBytes(total)}
        </span>
      </button>

      {open && (
        <>
          <p className="muted media__note">
            Эти файлы лежат на сервере, но не числятся ни за одним медиа. Они занимают
            место и раздаются по прямым ссылкам. Отнесите их к медиа — или удалите.
          </p>
          <ul className="file-list">
            {files.map((f) => (
              <div key={f.path} className="unrecognized__item">
                <FileRow file={f} onDelete={disabled ? undefined : onDelete} />
                {media.length > 0 && !disabled && (
                  <label className="unrecognized__assign">
                    <span>Отнести к медиа</span>
                    <select
                      defaultValue=""
                      onChange={(e) => {
                        if (e.target.value) onAssign(f.path, e.target.value);
                      }}
                    >
                      <option value="">— выберите —</option>
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
