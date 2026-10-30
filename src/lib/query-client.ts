import { QueryClient } from "@tanstack/react-query";
import type { IpcError } from "./ipc";

// Makes `query.error` typed as IpcError everywhere.
declare module "@tanstack/react-query" {
  interface Register {
    defaultError: IpcError;
  }
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        // Backend errors are deterministic (a port is busy, a file is missing). Retrying only
        // delays the error message, so surface it at once and let the user press "Try again".
        retry: false,
        refetchOnWindowFocus: false,
      },
    },
  });
}
