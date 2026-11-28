import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { Settings } from "@/bindings/Settings";
import { ipc } from "@/lib/ipc";

export const SETTINGS_KEY = ["settings"] as const;
const LOGIN_KEY = ["settings", "launchAtLogin"] as const;

export function useSettings() {
  return useQuery({ queryKey: SETTINGS_KEY, queryFn: ipc.settingsGet });
}

export function useSaveSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: Settings) => ipc.settingsSet(settings),
    onSuccess: (view) => {
      queryClient.setQueryData(SETTINGS_KEY, view);
      // The default versions change what projects resolve to.
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

export function useLaunchAtLogin() {
  return useQuery({ queryKey: LOGIN_KEY, queryFn: ipc.launchAtLoginGet });
}

export function useSetLaunchAtLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) => ipc.launchAtLoginSet(enabled),
    onSuccess: (enabled) => {
      queryClient.setQueryData(LOGIN_KEY, enabled);
    },
  });
}
