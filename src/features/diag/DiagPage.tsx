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
import { NoServer } from "../shared/NoServer";

export function DiagPage() {
  const server = useActiveServer();

  if (!server) return <NoServer />;
  return <DiagScreen serverId={server.id} />;
}
