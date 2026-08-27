/**
 * T329 — оформление: тема, маскот, движение (FR-100, FR-103).
 *
 * Все три поля лежали в `settings.rs` с самой вехи A, и ни одно не было доступно человеку:
 * настройка, которую нельзя изменить, — это не настройка, а значение по умолчанию с лишним
 * полем в базе.
 *
 * **Последствия написаны рядом с переключателями, а не в подсказке.** «Выключить движение»
 * ничего не говорит тому, у кого от него укачивает, — а «переходы между разделами и
 * шевеление маскота» говорит.
 */

import { ErrorNotice } from "../shared/ErrorNotice";
import { useSettings } from "../../app/settings";
import { useLang, useT, LANGUAGES } from "../../shared/i18n";
import { useTheme, type ThemeChoice } from "../../app/theme";

const THEMES: ThemeChoice[] = ["light", "dark", "system"];

export function Appearance() {
  const t = useT();
  const words = t.ui.appearance;
  const { lang, setLang } = useLang();
  const { choice, setChoice } = useTheme();
  const { settings, update, error } = useSettings();

  return (
    <div className="appearance">
      <h2>{words.title}</h2>
      {error && <ErrorNotice error={error} />}

      <fieldset>
        <legend>{words.theme}</legend>
        {THEMES.map((it) => (
          <label key={it}>
            <input
              type="radio"
              name="theme"
              value={it}
              checked={choice === it}
              onChange={() => setChoice(it)}
            />
            {words[`theme${it[0].toUpperCase()}${it.slice(1)}` as keyof typeof words] as string}
          </label>
        ))}
      </fieldset>

      <fieldset>
        <legend>{words.language}</legend>
        {LANGUAGES.map((it) => (
          <label key={it.lang}>
            <input
              type="radio"
              name="language"
              value={it.lang}
              checked={lang === it.lang}
              onChange={() => setLang(it.lang)}
            />
            {/* Каждый язык назван на себе самом: тот, кто попал в язык, которого не
                читает, должен суметь найти свой. */}
            {it.label}
          </label>
        ))}
      </fieldset>

      <fieldset>
        <legend>{words.mascot}</legend>
        <label>
          <input
            type="checkbox"
            checked={settings?.mascot ?? true}
            disabled={settings === null}
            onChange={(e) => update({ mascot: e.target.checked })}
            data-testid="mascot-switch"
          />
          {words.mascotOn}
        </label>
        <p className="appearance__means">{words.mascotMeans}</p>
      </fieldset>

      <fieldset>
        <legend>{words.animations}</legend>
        <label>
          <input
            type="checkbox"
            checked={settings?.animations ?? true}
            disabled={settings === null}
            onChange={(e) => update({ animations: e.target.checked })}
            data-testid="animations-switch"
          />
          {words.animationsOn}
        </label>
        <p className="appearance__means">{words.animationsMeans}</p>
      </fieldset>
    </div>
  );
}
