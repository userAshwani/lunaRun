import { useQuery } from "@tanstack/react-query";
import { ipc } from "@/lib/ipc";

export const APP_INFO_KEY = ["appInfo"] as const;

export function useAppInfo() {
  return useQuery({
    queryKey: APP_INFO_KEY,
    queryFn: ipc.appInfo,
    // Version, platform and data folder do not change while the app runs.
    staleTime: Infinity,
  });
}
