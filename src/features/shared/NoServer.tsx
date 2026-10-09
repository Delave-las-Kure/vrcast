/**
 * T709 — what a section says when it has no server to work with.
 *
 * «No server selected» was said on every section, and to somebody who has not added a server
 * yet it reads as if one were there and merely unchosen — they go looking for a choice that
 * does not exist. Two different situations, two different lines: with no profiles at all the
 * way on is to add one; with several and none active (the only one is active by itself, T708)
 * it is to choose. Either way the link goes to «Servers», where both are done.
 */

import { Link } from "react-router-dom";

import { useServers } from "../servers/store";
import { useT } from "../../shared/i18n";

export function NoServer({ className = "muted" }: { className?: string }) {
  const t = useT();
  const none = useServers((s) => s.profiles.length === 0);
  return (
    <p className={className}>
      {none ? t.ui.common.noServers : t.ui.common.noActiveServer}{" "}
      <Link to="/servers">{t.ui.common.toServers}</Link>
    </p>
  );
}
