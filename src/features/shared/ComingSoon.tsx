/**
 * A placeholder for a section that does not exist yet.
 *
 * An honest one: it names the phase and what will appear. A blank screen with no
 * explanation looks broken, and "coming soon" says nothing at all.
 */

import { useT } from "../../shared/i18n";

export function ComingSoon({
  title,
  phase,
  what,
  fallback,
}: {
  title: string;
  phase: string;
  what: string;
  /** What to use while the section is missing. */
  fallback?: string;
}) {
  const t = useT();

  return (
    <div className="coming-soon">
      <h1>{title}</h1>
      <p className="coming-soon__phase">{phase}</p>
      <p>{what}</p>
      {fallback && (
        <p className="coming-soon__fallback">
          <strong>{t.ui.comingSoon.useMeanwhile}</strong> {fallback}
        </p>
      )}
    </div>
  );
}
