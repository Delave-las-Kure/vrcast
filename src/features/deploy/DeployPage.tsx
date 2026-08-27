/**
 * Where the "deploy" link on a server card leads.
 *
 * The server comes from the address, not from "the active one": what gets deployed is usually
 * **not** the server being worked with — the active one is serving films, and the new one is
 * still empty. Taking the active one would deploy to the wrong machine, and the way that would
 * be discovered is by the serving stopping.
 */

import { useSearchParams } from "react-router-dom";

import { DeployScreen } from "./DeployScreen";
import { useT } from "../../shared/i18n";

export function DeployPage() {
  const t = useT();
  const [params] = useSearchParams();
  const serverId = params.get("server");

  if (!serverId) return <p>{t.ui.servers.empty}</p>;
  return <DeployScreen serverId={serverId} />;
}
