/**
 * Состояние сервера, показатель за показателем (FR-070).
 *
 * **Оценка и числа стоят рядом.** «Внимание» само по себе не говорит человеку ничего: он не
 * может ни проверить его, ни возразить ему, ни решить, что делать. Формулировка приходит из
 * ядра кодом с подставленными значениями — «кеш раздачи всего 100 МБ из 1900, а смотрят
 * трое», — и ниже лежат сырые показания целиком, на случай если оценка неверна. Она бывает
 * неверна.
 *
 * **«Не выяснено» — своя оценка, не «норма».** В контейнере не видно ни настроек ядра, ни
 * диска, и панель, назвавшая это нормой, отчиталась бы о проверенном там, где ничего не
 * проверялось.
 */

import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import type { Health, Rated, Rating, Reading } from "../../shared/contract";

/** Как назвать оценку. */
function ratingWord(rating: Rating, words: Record<string, string>): string {
  if (rating === "fine") return words.ratingFine;
  if (rating === "watch") return words.ratingWatch;
  if (rating === "trouble") return words.ratingTrouble;
  return words.ratingUnknown;
}

/** Как назвать сам показатель. */
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
            {/* Числа, на которых оценка держится. Не подсказка при наведении: то, что
                видно только при наведении, не видно. */}
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
