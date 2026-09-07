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
import { BatchScreen } from "../features/batch/BatchScreen";
import { useActiveServer, useServers } from "../features/servers/store";
import { LimitsList } from "../features/viewers/LimitsList";
import { Appearance } from "../features/settings/Appearance";
import { DiagPage } from "../features/diag/DiagPage";
import { ViewersScreen } from "../features/viewers/ViewersScreen";
import { TasksPanel } from "../features/tasks/TasksPanel";
import { LeaveConfirm } from "../features/tasks/LeaveConfirm";
import { UploadScreen } from "../features/upload/UploadScreen";
import { ipc } from "../shared/ipc";
import { LanguageProvider, useT } from "../shared/i18n";
import { useTaskNotifications, useTrayNotice } from "../features/tasks/notifications";
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
  // T509 — whether there is a first-run wizard to send somebody to. `profiles.length === 0`
  // means "nobody has a server profile yet", and that is only trustworthy once the initial
  // load has actually finished (`!loading`): during the load itself the list reads as
  // empty regardless of what is really on disk, and sending a person to `/servers` on that
  // basis would flash the wizard at everyone for a moment, on every start.
  const { profiles, loading: profilesLoading, reload: reloadServers } = useServers();

  useEffect(() => {
    // Read early enough that `profiles`/`loading` are settled by the time `/` is first
    // decided. `ServerList.tsx` already calls this on its own mount, but that is too late
    // here: the redirect below has to know the answer before that screen ever renders.
    void reloadServers();
  }, [reloadServers]);

  // Until the settings have been read, movement is on: that is the default, and one stray
  // flicker is better than looking dead to somebody who left it turned on.
  const motion = useSettings().settings?.animations !== false;

  useEffect(() => {
    ipc
      .appVersions()
      .then((v) => setVersion(v.app))
      // The version is decoration; its absence is no reason for a full-screen error.
      .catch(() => setVersion(null));
  }, []);

  // The tray menu is worded here for the same reason as the notifications below (T395):
  // the core writes no words of its own, and the tray is the one place in it that puts text
  // on a screen. Handed over again on every change of language — a menu still reading
  // Russian after somebody switched to English is a menu that ignored them.
  useEffect(() => {
    void ipc
      .trayLabels({ show: t.ui.tray.show, quit: t.ui.tray.quit })
      // No tray on this desktop is an ordinary state, not a fault: the core has already
      // said so through `trayState`, and the close button behaves accordingly.
      .catch(() => undefined);
  }, [t]);

  // System notifications are worded here, not in the core: the core decides *whether*
  // to notify (only it knows the window is out of sight), the interface decides *what
  // it says*, in the language in use.
  useTaskNotifications();

  // Where the window went, the first time it goes into the tray (T399). Here rather than on
  // a screen: the window is hidden when it happens, and a listener living inside a section
  // would only hear it if that section were the one open.
  useTrayNotice();

  return (
    /*
     * **The switch lives on the whole layout, not on the content alone** (T516).
     *
     * It used to sit on `<main className="content">`, and the mascot is in the side panel —
     * outside it. So the setting whose own label promises "transitions between sections and
     * the mascot's movement" could not reach the second half of what it promised: the mascot
     * went on nodding, gated only by the system's own reduce-motion. Here it covers both.
     */
    <div className="layout" data-motion={motion ? "on" : "off"}>
      {/*
       * T400 — the question the tray's "Exit" now asks (FR-086).
       *
       * Mounted here, above the routes, on purpose: the menu item can be chosen from any
       * screen and while the window is hidden, and a listener living inside one section
       * would miss it from every other. It draws nothing until the core asks.
       */}
      <LeaveConfirm />
      <Sidebar version={version} />
      {/*
       * T326 — fades between sections (FR-101), which a setting can turn off (FR-103).
       *
       * The address is the key: without it React takes the contents for the same thing and the
       * fade happens once, on the first render. The switch itself is read by CSS from
       * `data-motion` on the layout above — the rule lives beside the animation and cannot
       * drift away from it — and the system's own "reduce motion" is checked in the same place
       * and outranks ours: what somebody has turned off system-wide is not for the application
       * to turn back on.
       */}
      <main className="content">
        <Routes key={location.pathname}>
          <Route
            path="/"
            element={
              // T509 — nobody with zero server profiles has any business landing on an
              // empty task panel with no hint at all of what to do next. `/servers`
              // already shows `t.ui.servers.empty` and the "Add server" button that opens
              // `SetupWizard` (`ServerList.tsx`) — nothing there needs fixing, only this
              // redirect needed pointing at it.
              //
              // While the very first load is still in flight nothing is rendered here at
              // all — not even a guess at `/tasks`. `<Route path="/">` only re-evaluates
              // its element while the address is still exactly "/"; a guessed redirect to
              // `/tasks` would move the address away and this decision would never run a
              // second time once the real answer came in, leaving a first-time person
              // stranded on the empty task panel this exists to route them away from.
              // Waiting here costs one blank instant behind the (already mounted) sidebar,
              // not a flicker between two screens.
              profilesLoading ? null : profiles.length === 0 ? (
                <Navigate to="/servers" replace />
              ) : (
                <Navigate to="/tasks" replace />
              )
            }
          />
          <Route path="/tasks" element={<TasksPanel />} />
          <Route path="/servers" element={<ServerList />} />
          <Route path="/library" element={<LibraryScreen />} />
          <Route path="/convert" element={<ConvertScreen />} />
          <Route path="/upload" element={<UploadScreen />} />
          <Route path="/about" element={<About />} />
          <Route path="/ladder" element={<LadderPage />} />
          <Route path="/batch" element={<BatchScreen />} />
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
    // Settings outermost: the theme and the language both come from there now, rather than
    // from two stores that drifted apart in silence (T324).
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
