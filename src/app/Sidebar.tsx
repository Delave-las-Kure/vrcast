import { NavLink } from "react-router-dom";
import { LANGUAGES, useLang, useT } from "../shared/i18n";
import { fill } from "../shared/i18n/render";
import { useTheme, type ThemeChoice } from "./theme";
import { Mascot } from "../features/mascot/Mascot";

/**
 * The sections, in the order work goes through them: server, library, doing, checking.
 *
 * The label is a key rather than a word: the sidebar is the one place every language
 * has to agree on, and a hard-coded word here would be the first thing to go stale.
 *
 * **`step` numbers the three that are one job done in order** — prepare the file, cut it
 * into qualities, send it. They used to be listed with sending before cutting, which is
 * neither the order they happen in nor the order they were built in, and a person following
 * the menu down the page was being led the wrong way round.
 */
export const SECTIONS = [
  { path: "/servers", key: "servers", ready: true },
  { path: "/library", key: "library", ready: true },
  { path: "/convert", key: "convert", ready: true, step: 1 },
  { path: "/ladder", key: "ladder", ready: true, step: 2 },
  { path: "/upload", key: "upload", ready: true, step: 3 },
  { path: "/viewers", key: "viewers", ready: true },
  { path: "/limits", key: "limits", ready: true },
  { path: "/diagnostics", key: "diagnostics", ready: true },
  { path: "/tasks", key: "tasks", ready: true },
  { path: "/appearance", key: "appearance", ready: true },
] as const;

const THEME_ORDER: ThemeChoice[] = ["light", "dark", "system"];

export function Sidebar({ version }: { version: string | null }) {
  const { choice, setChoice } = useTheme();
  const { lang, setLang } = useLang();
  const t = useT();

  return (
    <nav className="sidebar" aria-label={t.ui.sidebar.sections}>
      <div className="sidebar__brand">
        <span className="sidebar__title">VRCast Studio</span>
        {version && (
          <NavLink to="/about" className="sidebar__version" title={t.ui.sidebar.aboutTitle}>
            {fill(t.ui.sidebar.version, { version }, t, lang)}
          </NavLink>
        )}
      </div>

      <ul className="sidebar__list">
        {SECTIONS.map((s) => (
          <li key={s.path}>
            <NavLink
              to={s.path}
              className={({ isActive }) =>
                `sidebar__link${isActive ? " sidebar__link--active" : ""}`
              }
            >
              <span>
                {"step" in s
                  ? fill(t.ui.sidebar.step, { n: s.step, name: t.ui.sections[s.key] }, t, lang)
                  : t.ui.sections[s.key]}
              </span>
              {/* A section that does not exist yet is marked honestly rather than
                  being left to look finished. */}
              {!s.ready && (
                <span className="sidebar__soon" title={t.ui.sidebar.notReady}>
                  ·
                </span>
              )}
            </NavLink>
          </li>
        ))}
      </ul>

      <Mascot />

      <div className="sidebar__footer">
        <label className="sidebar__theme">
          <span>{t.ui.common.language}</span>
          <select
            value={lang}
            onChange={(e) => setLang(e.target.value as typeof lang)}
            aria-label={t.ui.common.language}
          >
            {LANGUAGES.map((l) => (
              // Each language is named in itself, never translated: someone who has
              // landed in a language they cannot read must still find their own.
              <option key={l.lang} value={l.lang}>
                {l.label}
              </option>
            ))}
          </select>
        </label>

        <label className="sidebar__theme">
          <span>{t.ui.appearance.theme}</span>
          <select
            value={choice}
            onChange={(e) => setChoice(e.target.value as ThemeChoice)}
            aria-label={t.ui.appearance.theme}
          >
            {THEME_ORDER.map((k) => (
              <option key={k} value={k}>
                {t.ui.common.theme[k]}
              </option>
            ))}
          </select>
        </label>
      </div>
    </nav>
  );
}
