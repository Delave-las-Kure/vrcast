/**
 * How «Video» reads the library to tell whether the server answers (tour 0.3.0, the owner).
 *
 * The first look at a server starts a refresh behind the last known library, so that a
 * server gone down is found out. **No more than once a minute per server**: coming back to
 * the section, or switching between sections, takes the library as already known and asks
 * the server nothing — every refresh is an SSH login and a walk over the video directory.
 */

import type { LibraryView } from "../../shared/contract";
import { ipc } from "../../shared/ipc";

/** The shortest time between two refreshes «Video» starts for one server. */
export const REFRESH_EVERY_MS = 60_000;

/** When «Video» last started a refresh, by server. */
const lastRefresh = new Map<string, number>();

/** The library for the server-down line: refreshed at most once a minute, known otherwise. */
export function libraryForVideo(serverId: string, now = Date.now()): Promise<LibraryView> {
  const last = lastRefresh.get(serverId);
  if (last !== undefined && now - last < REFRESH_EVERY_MS) return ipc.libraryKnown(serverId);
  lastRefresh.set(serverId, now);
  return ipc.libraryList(serverId, false);
}

/** Forget when refreshes were started — for tests, which each start from nothing. */
export function forgetRefreshes(): void {
  lastRefresh.clear();
}
