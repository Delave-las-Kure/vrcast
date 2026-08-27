/**
 * The state of the server, reading by reading (FR-070).
 *
 * **The verdict and the numbers stand together.** "Worth a look" on its own tells a person
 * nothing: they can neither check it, nor argue with it, nor decide what to do. The wording
 * comes from the core as a code with values filled in — "the serving cache is only 100 MB of
 * 1900, and three people are watching" — and the raw readings lie in full underneath, in case
 * the verdict is wrong. It is sometimes wrong.
 *
 * **"Not determined" is a verdict of its own, not "fine".** In a container neither the kernel
 * settings nor the disk can be seen, and a panel calling that fine would be reporting
 * something checked where nothing was checked at all.
 */

import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import type { Health, Rated, Rating, Reading } from "../../shared/contract";

/** What to call a verdict. */
function ratingWord(rating: Rating, words: Record<string, string>): string {
  if (rating === "fine") return words.ratingFine;
  if (rating === "watch") return words.ratingWatch;
  if (rating === "trouble") return words.ratingTrouble;
  return words.ratingUnknown;
}

/** What to call the reading itself. */
function readingWord(about: Reading, words: Record<string, string>): string {
  const key = "reading" + about.replace(/(^|_)([a-z])/g, (_, __, c: string) => c.toUpperCase());
  return words[key] ?? about;
}

export function HealthPanel({ health }: { health: Health }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.diag as unknown as Record<string, string>;

  return (
    <section className="diag-health">
      <h3>{words.healthTitle}</h3>
      <p className={`diag-worst diag-rating-${health.worst}`} data-testid="diag-worst">
        {ratingWord(health.worst, words)}
      </p>

      <ul className="diag-readings">
        {health.readings.map((reading: Rated) => (
          <li
            key={reading.about}
            className={`diag-rating-${reading.rating}`}
            data-testid={`reading-${reading.about}`}
            data-rating={reading.rating}
          >
            <span className="diag-reading-name">{readingWord(reading.about, words)}</span>
            <span className="diag-reading-mark">{ratingWord(reading.rating, words)}</span>
            {/* The numbers the verdict rests on. Not a tooltip: what can only be seen by
                hovering cannot be seen. */}
            <span className="diag-reading-say">{renderDetail(reading.say, t, lang)}</span>
          </li>
        ))}
      </ul>

      <details className="diag-raw">
        <summary>{words.rawTitle}</summary>
        <p className="diag-hint">{words.rawHint}</p>
        <pre data-testid="diag-raw">{JSON.stringify(health.snapshot, null, 2)}</pre>
      </details>
    </section>
  );
}
