/**
 * T123 — showing the verdict of the playback check (FR-027).
 *
 * A file that did not pass must not be offered for upload, and the person has to
 * be able to see that it did not. A broken encode opens fine, reports the right
 * duration and the right frame count — nothing about it looks wrong until someone
 * is watching it.
 *
 * The decoder's own words are shown verbatim. "Invalid NAL unit size" is cryptic,
 * but it can be searched for; "the file is broken" cannot.
 */

import type { Validation } from "../../shared/contract";

export function ValidationResult({ result }: { result: Validation }) {
  return (
    <section
      className={`notice ${result.ok ? "notice--ok" : "notice--error"}`}
      role={result.ok ? "status" : "alert"}
    >
      <div className="notice__body">
        <strong className="notice__message">
          {result.ok
            ? "Файл воспроизводится целиком — можно заливать."
            : "Файл не прошёл проверку воспроизведения. Заливать его нельзя: у зрителя он развалится там же, где развалился здесь."}
        </strong>

        {result.problems.length > 0 && (
          <>
            <p className="notice__hint">Что сказал декодер:</p>
            <ul className="notice__list">
              {result.problems.map((p) => (
                <li key={p}>
                  <code>{p}</code>
                </li>
              ))}
            </ul>
          </>
        )}

        {result.ignored.length > 0 && (
          /*
           * Shown rather than hidden. These are the muxer's complaints about
           * timestamps: they do not come from the decoder and do not mean the file
           * is bad. Hiding them would leave someone wondering later why a file with
           * warnings was accepted.
           */
          <details className="notice__details">
            <summary>
              Замечаний к меткам времени: {result.ignored.length} — на воспроизведение
              не влияют
            </summary>
            <ul className="notice__list">
              {result.ignored.map((p) => (
                <li key={p}>
                  <code>{p}</code>
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </section>
  );
}
