/**
 * T027 — оболочка приложения: разделы, оформление, подписка на поток событий.
 *
 * Работают разделы задач, серверов, библиотеки и заливки. Остальные честно говорят, в какой
 * фазе появятся и чем пользоваться до тех пор, — пустой экран без объяснения
 * выглядит поломкой.
 */

import { useEffect, useState } from "react";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { LibraryScreen } from "../features/library/LibraryScreen";
import { ServerList } from "../features/servers/ServerList";
import { ComingSoon } from "../features/shared/ComingSoon";
import { TasksPanel } from "../features/tasks/TasksPanel";
import { UploadScreen } from "../features/upload/UploadScreen";
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
          <Route path="/servers" element={<ServerList />} />
          <Route path="/library" element={<LibraryScreen />} />
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
          <Route path="/upload" element={<UploadScreen />} />
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
