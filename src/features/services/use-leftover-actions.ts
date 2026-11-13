import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LunaError } from "@/bindings/LunaError";
import { useToast } from "@/components/toast-context";
import { IpcError, ipc, toLunaError } from "@/lib/ipc";
import { LEFTOVERS_KEY, SERVICES_KEY } from "./use-services";

export type LeftoverAction = "adopt" | "stop";

/**
 * Adopting or stopping a service an earlier session left running. Each takes a moment (adopting waits for the service
 * to answer, stopping gives it ten seconds to leave), so the row says what is under way, and a refusal stays next to
 * the service with its reason until the next attempt.
 */
export function useLeftoverActions(nameOf: (id: string) => string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const { t } = useTranslation();
  const [pending, setPending] = useState<ReadonlyMap<string, LeftoverAction>>(new Map());
  const [errors, setErrors] = useState<ReadonlyMap<string, LunaError>>(new Map());

  const run = useCallback(
    async (action: LeftoverAction, id: string) => {
      setErrors((current) => {
        const next = new Map(current);
        next.delete(id);
        return next;
      });
      setPending((current) => new Map(current).set(id, action));
      try {
        if (action === "adopt") await ipc.serviceAdopt(id);
        else await ipc.serviceStopLeftover(id);
        toast.show({ title: t(`services.leftovers.done.${action}`, { name: nameOf(id) }) });
      } catch (thrown) {
        const error = thrown instanceof IpcError ? thrown.error : toLunaError(thrown);
        setErrors((current) => new Map(current).set(id, error));
      } finally {
        setPending((current) => {
          const next = new Map(current);
          next.delete(id);
          return next;
        });
        void queryClient.invalidateQueries({ queryKey: LEFTOVERS_KEY });
        void queryClient.invalidateQueries({ queryKey: SERVICES_KEY });
      }
    },
    [nameOf, queryClient, t, toast],
  );

  return {
    /** The leftovers whose command is under way, with which one. */
    pending,
    /** Why the last attempt on a leftover was refused. */
    errors,
    adopt: useCallback((id: string) => run("adopt", id), [run]),
    stop: useCallback((id: string) => run("stop", id), [run]),
  };
}
