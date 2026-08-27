/**
 * T324 — настройки в одном месте, а не в двух.
 *
 * **Два хранилища одного выбора расходятся молча.** Тема жила в `localStorage`, язык — там
 * же, а в базе ядра всё это время лежали поля `theme` и `language`, которые никто не читал.
 * Человек видел бы одну тему, настройки показывали другую, и починить он этого не может,
 * потому что второго места не видит.
 *
 * Одно место — ядро. Не потому, что оно лучше, а потому, что оно одно на все окна и
 * переживает переустановку интерфейса: настройки лежат в той же базе, что и профили серверов,
 * и уезжают вместе с ней.
 *
 * **Записывает отсюда всё, что пишет настройки.** `settings_set` принимает объект целиком, и
 * два независимых писателя со своей копией затирали бы правки друг друга — тема, сохранённая
 * поверх устаревшего снимка, вернула бы старый язык. Поэтому копия одна и живёт здесь.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { ipc } from "../shared/ipc";
import type { AppError, Settings } from "../shared/contract";

interface SettingsContextValue {
  /** `null`, пока не прочитаны. Не «значения по умолчанию»: показать чужой выбор как свой —
   *  это моргнуть светлой темой в лицо тому, кто выбрал тёмную. */
  settings: Settings | null;
  update: (patch: Partial<Settings>) => void;
  /** Что не сохранилось. Экран оформления показывает это; остальным знать не обязательно. */
  error: AppError | null;
}

/**
 * Пустышка на случай, когда провайдера нет.
 *
 * Так живут проверки интерфейса: они поднимают один экран, а не всё приложение. Пустышка
 * названа пустышкой и ничего не хранит — молчаливого второго хранилища из неё не выйдет.
 */
const NOTHING: SettingsContextValue = {
  settings: null,
  update: () => {},
  error: null,
};

const SettingsContext = createContext<SettingsContextValue>(NOTHING);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .settingsGet()
      .then((got) => {
        if (alive) setSettings(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, []);

  const update = useCallback((patch: Partial<Settings>) => {
    setSettings((current) => {
      if (!current) return current;
      const merged = { ...current, ...patch };
      // Применяется сразу, а сохраняется следом. Ждать ответа базы, чтобы перекрасить
      // экран, — это задержка на переключателе, которую человек читает как «не нажалось»,
      // и он нажимает ещё раз.
      setError(null);
      ipc.settingsSet(merged).catch((e: AppError) => setError(e));
      return merged;
    });
  }, []);

  const value = useMemo(() => ({ settings, update, error }), [settings, update, error]);
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsContextValue {
  return useContext(SettingsContext);
}
