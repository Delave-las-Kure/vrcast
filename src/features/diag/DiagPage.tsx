/**
 * Diagnostics, as a section of the application.
 *
 * The server comes from the active one rather than from the address: people arrive here from
 * the sidebar, not from a server card — with a complaint that things are stuttering, not with
 * the thought of inspecting one particular server. If none is active, it says so: an empty
 * screen would be read as "all is well".
 */

import { DiagScreen } from "./DiagScreen";
import { useActiveServer } from "../servers/store";
import { useT } from "../../shared/i18n";

export function DiagPage() {
  const t = useT();
  const server = useActiveServer();

  if (!server) return <p>{t.ui.viewers.noServer}</p>;
  return <DiagScreen serverId={server.id} />;
}
