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
 *
 * T480 — and where it can, it says what it thinks belongs together. The core has been able
 * to work that out since milestone A (`domain::grouping`), and nothing called it until the
 * reachability guard found the module unreachable. The suggestion is offered on opening the
 * group, and it changes nothing: it names the files it would put together and why, and the
 * person still assigns them one at a time.
 */

import { useEffect, useState } from "react";
import type { FileView, GroupSuggestion, MediaView } from "../../shared/contract";
import { ipc } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { FileRow } from "./FileRow";

export function UnrecognizedGroup({
  files,
  media,
  serverId,
  onAssign,
  onDelete,
  disabled,
}: {
  files: FileView[];
  media: MediaView[];
  serverId: string;
  onAssign: (path: string, mediaId: string) => void;
  onDelete: (path: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [suggestion, setSuggestion] = useState<GroupSuggestion | null>(null);
  const t = useT();
  const { lang } = useLang();
  const total = files.reduce((sum, f) => sum + f.size_bytes, 0);

  // Asked for only when the group is opened, and never again for the same list: it costs a
  // round trip, and nobody who has not opened the group is waiting for the answer.
  useEffect(() => {
    if (!open || suggestion) return;
    let alive = true;
    ipc
      .librarySuggestGroups(serverId)
      .then((s) => {
        if (alive) setSuggestion(s);
      })
      // A suggestion that will not come is not worth spoiling the list over: the files are
      // the point, and this sits beside them.
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [open, suggestion, serverId]);

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
          {suggestion?.groups?.length ? (
            <div className="unrecognized__suggestion" data-testid="group-suggestion">
              <p className="muted">{t.ui.library.suggestionNote}</p>
              <ul>
                {suggestion.groups.map((g) => (
                  <li key={g.key}>
                    <strong>{g.suggested_title}</strong>{" "}
                    <span className="muted">
                      {fill(
                        t.ui.library.suggestionGroup,
                        { n: g.files.length, why: t.ui.library.groupReason[g.reason] },
                        t,
                        lang,
                      )}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
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
