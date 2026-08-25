/**
 * Shared state: the server profiles.
 *
 * Kept out of the screens because the active server is needed by the servers section
 * and by the library alike. Holding it in each screen separately means showing one
 * server's library while another is selected, sooner or later.
 */

import { create } from "zustand";
import type { AppError, ServerProfile } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";

interface ServersState {
  profiles: ServerProfile[];
  /** True only until the first load: refreshes happen without a spinner. */
  loading: boolean;
  error: AppError | null;
  reload: () => Promise<void>;
  setActive: (id: string) => Promise<void>;
  clearError: () => void;
}

export const useServers = create<ServersState>((set, get) => ({
  profiles: [],
  loading: true,
  error: null,

  reload: async () => {
    try {
      set({ profiles: await ipc.serversList(), error: null });
    } catch (e) {
      set({ error: toAppError(e) });
    } finally {
      set({ loading: false });
    }
  },

  setActive: async (id) => {
    try {
      await ipc.serverSetActive(id);
      await get().reload();
    } catch (e) {
      set({ error: toAppError(e) });
    }
  },

  clearError: () => set({ error: null }),
}));

/** The active profile. `null` when there are no servers, or none is chosen. */
export function useActiveServer(): ServerProfile | null {
  return useServers((s) => s.profiles.find((p) => p.is_active) ?? null);
}

/** Whether the profile is usable: without a confirmed fingerprint there is no connection. */
export function isReady(profile: ServerProfile): boolean {
  return profile.host_fingerprint !== null;
}
