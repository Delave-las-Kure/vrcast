/**
 * T027 — the application shell: sections, appearance, the stream of events.
 *
 * The task, server, library, preparation and upload sections work. The rest say
 * honestly which phase they arrive in and what to use until then — a blank screen with
 * no explanation looks broken.
 */

import { useEffect, useState } from "react";
import { HashRouter, Navigate, Route, Routes, useLocation } from "react-router-dom";
import { LibraryScreen } from "../features/library/LibraryScreen";
import { ServerList } from "../features/servers/ServerList";
import { About } from "../features/shared/About";
import { ConvertScreen } from "../features/convert/ConvertScreen";
import { DeployPage } from "../features/deploy/DeployPage";
import { LadderPage } from "../features/ladder/LadderScreen";
import { useActiveServer } from "../features/servers/store";
import { LimitsList } from "../features/viewers/LimitsList";
import { Appearance } from "../features/settings/Appearance";
import { DiagPage } from "../features/diag/DiagPage";
import { ViewersScreen } from "../features/viewers/ViewersScreen";
import { TasksPanel } from "../features/tasks/TasksPanel";
import { UploadScreen } from "../features/upload/UploadScreen";
import { ipc } from "../shared/ipc";
import { LanguageProvider, useT } from "../shared/i18n";
import { useTaskNotifications } from "../features/tasks/notifications";
import { Sidebar } from "./Sidebar";
import { SettingsProvider, useSettings } from "./settings";
import { ThemeProvider } from "./theme";

function AppShell() {
  const [version, setVersion] = useState<string | null>(null);
  const t = useT();
  // Which server the limits belong to. Read here rather than inside the screen so that a
  // person who has not chosen one is told so, instead of being shown an empty list that
  // looks like "nothing is capped".
  const activeServer = useActiveServer()?.id ?? null;
  const location = useLocation();
  // Пока настройки не прочитаны — движение включено: оно включено по умолчанию, и
  // дёрнуться один раз лучше, чем показаться неживым тому, кто его оставил.
  const motion = useSettings().settings?.animations !== false;

  useEffect(() => {
    ipc
      .appVersions()
      .then((v) => setVersion(v.app))
      // The version is decoration; its absence is no reason for a full-screen error.
      .catch(() => setVersion(null));
  }, []);

  // System notifications are worded here, not in the core: the core decides *whether*
  // to notify (only it knows the window is out of sight), the interface decides *what
  // it says*, in the language in use.
  useTaskNotifications();

  return (
    <div className="layout">
      <Sidebar version={version} />
      {/*
       * T326 — плавные переходы между разделами (FR-101), отключаемые настройкой (FR-103).
       *
       * Ключом стоит адрес: без него React считает содержимое тем же самым и переход
       * происходит один раз, при первой отрисовке. Признак `data-motion` читает CSS —
       * правило живёт рядом с самой анимацией и не разъезжается с ней, а системное
       * «уменьшить движение» проверяется там же и перевешивает: то, что человек выключил
       * в системе, приложение включать обратно не вправе.
       */}
      <main className="content" data-motion={motion ? "on" : "off"}>
        <Routes key={location.pathname}>
          <Route path="/" element={<Navigate to="/tasks" replace />} />
          <Route path="/tasks" element={<TasksPanel />} />
          <Route path="/servers" element={<ServerList />} />
          <Route path="/library" element={<LibraryScreen />} />
          <Route path="/convert" element={<ConvertScreen />} />
          <Route path="/upload" element={<UploadScreen />} />
          <Route path="/about" element={<About />} />
          <Route path="/ladder" element={<LadderPage />} />
          <Route path="/viewers" element={<ViewersScreen />} />
          <Route path="/deploy" element={<DeployPage />} />
          <Route
            path="/limits"
            element={
              activeServer ? <LimitsList serverId={activeServer} /> : <p>{t.ui.viewers.noServer}</p>
            }
          />
          <Route path="/diagnostics" element={<DiagPage />} />
          <Route path="/appearance" element={<Appearance />} />
        </Routes>
      </main>
    </div>
  );
}

export default function App() {
  return (
    // Настройки снаружи всех: и тема, и язык теперь берутся оттуда, а не из двух
    // хранилищ, которые расходились молча (T324).
    <SettingsProvider>
      <LanguageProvider>
        <ThemeProvider>
          <HashRouter>
            <AppShell />
          </HashRouter>
        </ThemeProvider>
      </LanguageProvider>
    </SettingsProvider>
  );
}
