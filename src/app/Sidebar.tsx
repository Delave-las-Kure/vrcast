import { NavLink } from "react-router-dom";
import { useTheme, type ThemeChoice } from "./theme";

/** Разделы приложения. Порядок — по ходу работы: сервер, библиотека, дела, разбор. */
export const SECTIONS = [
  { path: "/servers", label: "Серверы", ready: false },
  { path: "/library", label: "Библиотека", ready: false },
  { path: "/convert", label: "Подготовка", ready: false },
  { path: "/upload", label: "Заливка", ready: false },
  { path: "/ladder", label: "Качества", ready: false },
  { path: "/viewers", label: "Зрители", ready: false },
  { path: "/limits", label: "Ограничения", ready: false },
  { path: "/diagnostics", label: "Диагностика", ready: false },
  { path: "/tasks", label: "Задачи", ready: true },
] as const;

const THEME_LABEL: Record<ThemeChoice, string> = {
  light: "Светлая",
  dark: "Тёмная",
  system: "Как в системе",
};

export function Sidebar({ version }: { version: string | null }) {
  const { choice, setChoice } = useTheme();

  return (
    <nav className="sidebar" aria-label="Разделы">
      <div className="sidebar__brand">
        <span className="sidebar__title">VRCast Studio</span>
        {version && <span className="sidebar__version">версия {version}</span>}
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
              <span>{s.label}</span>
              {/* Раздел, которого ещё нет, помечен честно, а не выглядит рабочим. */}
              {!s.ready && (
                <span className="sidebar__soon" title="Ещё не сделано">
                  ·
                </span>
              )}
            </NavLink>
          </li>
        ))}
      </ul>

      <div className="sidebar__footer">
        <label className="sidebar__theme">
          <span>Оформление</span>
          <select
            value={choice}
            onChange={(e) => setChoice(e.target.value as ThemeChoice)}
            aria-label="Оформление"
          >
            {(Object.keys(THEME_LABEL) as ThemeChoice[]).map((k) => (
              <option key={k} value={k}>
                {THEME_LABEL[k]}
              </option>
            ))}
          </select>
        </label>
      </div>
    </nav>
  );
}
