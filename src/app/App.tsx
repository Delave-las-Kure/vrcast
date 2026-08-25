/**
 * T027 — the application shell: sections, appearance, the stream of events.
 *
 * The task, server, library, preparation and upload sections work. The rest say
 * honestly which phase they arrive in and what to use until then — a blank screen with
 * no explanation looks broken.
 */

import { useEffect, useState } from "react";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { LibraryScreen } from "../features/library/LibraryScreen";
import { ServerList } from "../features/servers/ServerList";
import { About } from "../features/shared/About";
import { ConvertScreen } from "../features/convert/ConvertScreen";
import { ComingSoon } from "../features/shared/ComingSoon";
import { TasksPanel } from "../features/tasks/TasksPanel";
import { UploadScreen } from "../features/upload/UploadScreen";
import { ipc } from "../shared/ipc";
import { LanguageProvider, useT } from "../shared/i18n";
import { useTaskNotifications } from "../features/tasks/notifications";
import { Sidebar } from "./Sidebar";
import { ThemeProvider } from "./theme";

function AppShell() {
  const [version, setVersion] = useState<string | null>(null);
  const t = useT();

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

  const soon = t.ui.comingSoon;

  return (
    <div className="layout">
      <Sidebar version={version} />
      <main className="content">
        <Routes>
          <Route path="/" element={<Navigate to="/tasks" replace />} />
          <Route path="/tasks" element={<TasksPanel />} />
          <Route path="/servers" element={<ServerList />} />
          <Route path="/library" element={<LibraryScreen />} />
          <Route path="/convert" element={<ConvertScreen />} />
          <Route path="/upload" element={<UploadScreen />} />
          <Route path="/about" element={<About />} />
          <Route
            path="/ladder"
            element={
              <ComingSoon
                title={t.ui.sections.ladder}
                phase={soon.ladder.phase}
                what={soon.ladder.what}
                fallback={soon.ladder.fallback}
              />
            }
          />
          <Route
            path="/viewers"
            element={
              <ComingSoon
                title={t.ui.sections.viewers}
                phase={soon.viewers.phase}
                what={soon.viewers.what}
                fallback={soon.viewers.fallback}
              />
            }
          />
          <Route
            path="/limits"
            element={
              <ComingSoon
                title={t.ui.sections.limits}
                phase={soon.limits.phase}
                what={soon.limits.what}
                fallback={soon.limits.fallback}
              />
            }
          />
          <Route
            path="/diagnostics"
            element={
              <ComingSoon
                title={t.ui.sections.diagnostics}
                phase={soon.diagnostics.phase}
                what={soon.diagnostics.what}
                fallback={soon.diagnostics.fallback}
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
    <LanguageProvider>
      <ThemeProvider>
        <HashRouter>
          <AppShell />
        </HashRouter>
      </ThemeProvider>
    </LanguageProvider>
  );
}
