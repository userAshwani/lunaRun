import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "@/lib/ipc";

export const ONBOARDING_KEY = ["onboarding"] as const;

/** What the first-run wizard shows: whether it is the first run, the data folder, the checks and the starter pack. */
export function useOnboarding() {
  return useQuery({ queryKey: ONBOARDING_KEY, queryFn: ipc.onboardingGet });
}

/** Finish or skip the wizard: it is not shown again. */
export function useCompleteOnboarding() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ipc.onboardingComplete,
    onSettled: () => queryClient.invalidateQueries({ queryKey: ONBOARDING_KEY }),
  });
}

/** Whether a folder can be the data folder. Writes nothing. */
export function useCheckHome() {
  return useMutation({ mutationFn: (path: string) => ipc.onboardingCheckHome(path) });
}

/** Use the folder from the next start, then start again. */
export function useChooseHomeAndRestart() {
  return useMutation({
    mutationFn: async (path: string) => {
      await ipc.onboardingChooseHome(path);
      await ipc.appRestart();
    },
  });
}
