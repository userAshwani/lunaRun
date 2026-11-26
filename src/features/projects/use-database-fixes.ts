import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { PortChange } from "@/bindings/PortChange";
import { ipc } from "@/lib/ipc";
import { detectKey } from "./use-projects";
import { DATABASE_KEY } from "./use-project-run";

/** Under the key of the project's database settings, so what refreshes those (a project started or stopped) refreshes these. */
const envPortKey = (projectId: string) => [...DATABASE_KEY, projectId, "envPort"] as const;
const statusKey = (projectId: string) => [...DATABASE_KEY, projectId, "status"] as const;

/** The one line of `.env` that would be changed to connect to the database lunaRun runs, or `null` when nothing needs it. */
export function useEnvPortChange(projectId: string) {
  return useQuery({
    queryKey: envPortKey(projectId),
    queryFn: () => ipc.projectEnvPortPreview(projectId),
    // The file can be edited at any time: ask again whenever the page is shown.
    staleTime: 0,
  });
}

/** Make the previewed change: the backup, then the line. The checklist is read again afterwards (the port it shows changed). */
export function useApplyEnvPort(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (change: PortChange) => ipc.projectEnvPortApply(projectId, change),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: envPortKey(projectId) });
      void queryClient.invalidateQueries({ queryKey: detectKey(projectId) });
    },
  });
}

/** Whether the database the `.env` names is there. */
export function useDatabaseStatus(projectId: string) {
  return useQuery({
    queryKey: statusKey(projectId),
    queryFn: () => ipc.projectDatabaseStatus(projectId),
    staleTime: 0,
  });
}

export function useCreateDatabase(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => ipc.projectDatabaseCreate(projectId),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: statusKey(projectId) });
    },
  });
}
