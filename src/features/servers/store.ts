/**
 * Общее состояние: профили серверов.
 *
 * Вынесено из экранов потому, что активный сервер нужен и разделу серверов,
 * и библиотеке. Держать его в каждом экране отдельно значит однажды показать
 * библиотеку одного сервера, когда выбран другой.
 */

import { create } from "zustand";
import type { AppError, ServerProfile } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";

interface ServersState {
  profiles: ServerProfile[];
  /** Истина только до первой загрузки: обновления идут незаметно. */
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

/** Активный профиль. `null`, если серверов нет или ни один не выбран. */
export function useActiveServer(): ServerProfile | null {
  return useServers((s) => s.profiles.find((p) => p.is_active) ?? null);
}

/** Готов ли профиль к работе: без подтверждённого отпечатка подключения не будет. */
export function isReady(profile: ServerProfile): boolean {
  return profile.host_fingerprint !== null;
}
