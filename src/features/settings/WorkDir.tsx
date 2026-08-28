/**
 * T451 — where the working files go, said by the person rather than by the operating system.
 *
 * **What it costs to have no such setting.** A variant is one and a half to two gigabytes,
 * and until now it went to the system's temporary directory: `C:` on Windows, often a
 * memory-backed filesystem on Linux. Films live on the big disk; the system disk is the small
 * one. So a build ended hours in, out of space nobody agreed to spend — and on a tmpfs it
 * filled memory instead, which takes the machine with it.
 *
 * The default needs no setting: beside the source, on the disk that already holds a film and
 * therefore certainly fits one. This is for the person whose scratch disk is somewhere else.
 */

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useSettings } from "../../app/settings";
import { ipc } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

export function WorkDir() {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.appearance;
  const { settings, update } = useSettings();
  const chosen = settings?.work_dir ?? null;
  const [left, setLeft] = useState<{ files: number; bytes: number } | null>(null);

  /**
   * Point it somewhere new, and say what stayed behind (T453).
   *
   * Only when the old path was one somebody chose. Under the default the working folder sits
   * beside each film, so there is no single old place to look at — and beside the film is
   * where a person would look anyway, which is the whole argument for that default.
   */
  const moveTo = async (to: string | null) => {
    const from = chosen;
    update({ work_dir: to });
    setLeft(null);
    if (!from || from === to) return;
    try {
      const found = await ipc.workDirLeftovers(from);
      if (found.files > 0) setLeft(found);
    } catch {
      // Being unable to look is not a reason to spoil a settings screen.
    }
  };

  const pick = async () => {
    // A browser rather than a text field: a path typed by hand is a path with a typo in it,
    // and the mistake only shows up hours later when the build cannot write.
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string") await moveTo(picked);
  };

  return (
    <fieldset>
      <legend>{words.workDir}</legend>
      <p className="appearance__means">{words.workDirMeans}</p>
      <p data-testid="work-dir">{chosen ?? words.workDirDefault}</p>
      <button type="button" onClick={() => void pick()}>
        {words.workDirPick}
      </button>
      {/* Only when there is something to go back to. A control that does nothing teaches
          people the application is broken. */}
      {chosen && (
        <button type="button" onClick={() => void moveTo(null)}>
          {words.workDirReset}
        </button>
      )}
      {left && (
        <p role="note" data-testid="work-dir-left">
          {fill(
            words.workDirLeft,
            { files: left.files, mb: Math.round(left.bytes / 1_000_000) },
            t,
            lang,
          )}
        </p>
      )}
    </fieldset>
  );
}
