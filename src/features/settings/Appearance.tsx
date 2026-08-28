/**
 * T329 — appearance: the theme, the mascot, the movement (FR-100, FR-103).
 *
 * All three fields had been in `settings.rs` since milestone A, and not one of them could be
 * reached by a person: a setting nobody can change is not a setting, it is a default with an
 * extra column in the database.
 *
 * **What each switch costs is written beside it, not tucked into a tooltip.** "Turn off
 * movement" tells somebody who is made queasy by it nothing at all; "the fades between
 * sections and the mascot's fidgeting" tells them.
 */

import { ErrorNotice } from "../shared/ErrorNotice";
import { Forget } from "./Forget";
import { WorkDir } from "./WorkDir";
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
            {/* The same three labels as the sidebar's switch, from the same place. They used
                to be written twice — `appearance.themeLight` beside `common.theme.light` —
                and two copies of a word are two chances for the two switches to disagree
                (T463). */}
            {t.ui.common.theme[it]}
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
            {/* Each language is named in itself: somebody who has landed in a language they
                cannot read has to be able to find their own. */}
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

      {/* Not about appearance either, and here for the same reason: this is the only
          settings section there is. */}
      <WorkDir />

      {/* Not about appearance — but this is the only settings section there is, and there is
          nowhere deeper to hide "remove my data": here is where people will look for it. */}
      <Forget />
    </div>
  );
}
