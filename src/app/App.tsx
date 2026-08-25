/**
 * T027 — оболочка приложения: разделы, оформление, подписка на поток событий.
 *
 * Пока по-настоящему работает только раздел задач: он показывает то, что действительно
 * делает ядро. Остальные разделы честно говорят, в какой фазе появятся и чем пользоваться
 * до тех пор, — пустой экран без объяснения выглядит поломкой.
 */

import { useEffect, useState } from "react";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { ComingSoon } from "../features/shared/ComingSoon";
import { TasksPanel } from "../features/tasks/TasksPanel";
import { ipc } from "../shared/ipc";
import { Sidebar } from "./Sidebar";
import { ThemeProvider } from "./theme";

function AppShell() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    ipc
      .appVersions()
      .then((v) => setVersion(v.app))
      // Версия — украшение; её отсутствие не повод показывать ошибку на весь экран.
      .catch(() => setVersion(null));
  }, []);

  return (
    <div className="layout">
      <Sidebar version={version} />
      <main className="content">
        <Routes>
          <Route path="/" element={<Navigate to="/tasks" replace />} />
          <Route path="/tasks" element={<TasksPanel />} />
          <Route
            path="/servers"
            element={
              <ComingSoon
                title="Серверы"
                phase="Фаза 1"
                what="Мастер настройки, проверка подключения по шагам, подтверждение отпечатка сервера и переключение между несколькими серверами."
                fallback="вход по SSH и скрипты в .claude/skills/"
              />
            }
          />
          <Route
            path="/library"
            element={
              <ComingSoon
                title="Библиотека"
                phase="Фаза 1"
                what="Список медиа по названиям, все файлы каждого медиа с их параметрами, зрительские ссылки и место на диске сервера."
                fallback="ls на сервере и ссылки, составленные вручную"
              />
            }
          />
          <Route
            path="/convert"
            element={
              <ComingSoon
                title="Подготовка"
                phase="Фаза 3"
                what="Разбор исходника, выбор звуковой дорожки, перенос совместимого без пересжатия и обязательная проверка воспроизведения."
                fallback="скилл vrcast-convert"
              />
            }
          />
          <Route
            path="/upload"
            element={
              <ComingSoon
                title="Заливка"
                phase="Фаза 2"
                what="Передача с продолжением после обрыва, сверка контрольных сумм и ввод файла в раздачу одним неделимым действием."
                fallback="скилл vrcast-upload"
              />
            }
          />
          <Route
            path="/ladder"
            element={
              <ComingSoon
                title="Наборы качеств"
                phase="Фаза 5"
                what="Расчёт лесенки под конкретный источник и проверка отдачи каждого варианта."
                fallback="скилл vrcast-hls"
              />
            }
          />
          <Route
            path="/viewers"
            element={
              <ComingSoon
                title="Зрители"
                phase="Фаза 4"
                what="Кто смотрит прямо сейчас, откуда, с какой скоростью и у кого начались проблемы."
                fallback="скилл vrcast-diagnose"
              />
            }
          />
          <Route
            path="/limits"
            element={
              <ComingSoon
                title="Ограничения качества"
                phase="Фаза 6"
                what="Принудительное понижение качества для зрителя со слабым каналом — плеер VRChat сам этого не умеет."
                fallback="gen-slow-masters.py и правка Caddyfile вручную"
              />
            }
          />
          <Route
            path="/diagnostics"
            element={
              <ComingSoon
                title="Диагностика"
                phase="Фаза 8"
                what="Состояние сервера с оценками, разбор журналов и вероятная причина подвисаний."
                fallback="скилл vrcast-diagnose"
              />
            }
          />
        </Routes>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <HashRouter>
        <AppShell />
      </HashRouter>
    </ThemeProvider>
  );
}
