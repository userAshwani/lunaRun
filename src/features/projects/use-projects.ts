import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { Project } from "@/bindings/Project";
import type { ProjectDetection } from "@/bindings/ProjectDetection";
import type { ProjectPatch } from "@/bindings/ProjectPatch";
import { onJobDone } from "@/lib/events";
import { ipc } from "@/lib/ipc";

export const PROJECTS_KEY = ["projects", "list"] as const;
const DETECT_PREFIX = ["projects", "detect"] as const;
export const detectKey = (id: string) => [...DETECT_PREFIX, id] as const;

/** The projects the user added (the stored list: no project folder is opened to answer this). */
export function useProjects() {
  return useQuery({ queryKey: PROJECTS_KEY, queryFn: ipc.projectsList });
}

/**
 * Read a project's folder and say what it needs. A scan also updates what the list shows about the project (its
 * framework, the hash of its files), so the answer is put into the list's cache too.
 */
function useDetectFn() {
  const queryClient = useQueryClient();
  return async (id: string): Promise<ProjectDetection> => {
    const detection = await ipc.projectDetect(id);
    queryClient.setQueryData<Project[]>(PROJECTS_KEY, (list) =>
      list?.map((project) => (project.id === id ? detection.project : project)),
    );
    return detection;
  };
}

/** One project's scan. `refetch` is the Re-scan button. */
export function useProjectDetection(id: string, enabled = true) {
  const detect = useDetectFn();
  return useQuery({
    queryKey: detectKey(id),
    queryFn: () => detect(id),
    enabled,
    // The answer follows the files and what is installed: always ask again when the page is opened.
    staleTime: 0,
  });
}

/** The scan of every project, for the list's PHP and Node columns. One result per project, in the same order. */
export function useProjectDetections(projects: readonly Project[] | undefined) {
  const detect = useDetectFn();
  return useQueries({
    queries: (projects ?? []).map((project) => ({
      queryKey: detectKey(project.id),
      queryFn: () => detect(project.id),
      staleTime: 15_000,
    })),
  });
}

export function useAddProject() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => ipc.projectAdd(path),
    onSuccess: (added) => {
      queryClient.setQueryData(detectKey(added.project.id), added);
      return queryClient.invalidateQueries({ queryKey: PROJECTS_KEY });
    },
  });
}

export function useRemoveProject() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => ipc.projectRemove(id),
    onSuccess: (_none, id) => {
      queryClient.removeQueries({ queryKey: detectKey(id) });
      return queryClient.invalidateQueries({ queryKey: PROJECTS_KEY });
    },
  });
}

export function useUpdateProject() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: ProjectPatch }) =>
      ipc.projectUpdate(id, patch),
    onSuccess: (project) => {
      queryClient.setQueryData<Project[]>(PROJECTS_KEY, (list) =>
        list?.map((existing) => (existing.id === project.id ? project : existing)),
      );
      // A binding or a document root changes what the project resolves to.
      return queryClient.invalidateQueries({ queryKey: detectKey(project.id) });
    },
  });
}

/**
 * An install or an uninstall changes what every project resolves to, so when any job ends the scans are asked for
 * again (the list and the detail page both read them).
 */
export function useRescanWhenRuntimesChange() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;
    void onJobDone(() => {
      void queryClient.invalidateQueries({ queryKey: DETECT_PREFIX });
    }).then((off) => {
      if (lifecycle.disposed) off();
      else unsubscribe = off;
    });
    return () => {
      lifecycle.disposed = true;
      unsubscribe?.();
    };
  }, [queryClient]);
}
