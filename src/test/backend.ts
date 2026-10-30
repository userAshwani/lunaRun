// A fake Tauri backend for component tests. A test file must first mock the two Tauri modules:
//
//   vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
//   vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
//
// then call `resetBackend()` in `beforeEach`, `respondTo({...})` to answer commands, and `emitEvent` to
// push a backend event to everything that called `listen`.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { vi } from "vitest";

export const invokeMock = vi.mocked(invoke);
export const isTauriMock = vi.mocked(isTauri);
const listenMock = vi.mocked(listen);

type Listener = (event: { payload: unknown }) => void;
const listeners = new Map<string, Set<Listener>>();

/** Reset every mock and pretend to run inside the desktop app. Call from `beforeEach`. */
export function resetBackend(): void {
  vi.resetAllMocks();
  listeners.clear();
  isTauriMock.mockReturnValue(true);
  listenMock.mockImplementation((name: string, handler: unknown) => {
    const set = listeners.get(name) ?? new Set<Listener>();
    set.add(handler as Listener);
    listeners.set(name, set);
    return Promise.resolve(() => {
      set.delete(handler as Listener);
    });
  });
}

/** Answer commands. Each handler returns the value, or a rejected promise to simulate a backend error. */
export function respondTo(handlers: Record<string, (args?: unknown) => unknown>): void {
  invokeMock.mockImplementation((command: string, args?: unknown) => {
    const handler = handlers[command];
    if (!handler) return Promise.reject(new Error(`unexpected command: ${command}`));
    return Promise.resolve(handler(args));
  });
}

/** Deliver a backend event to every subscriber, as Tauri would. */
export function emitEvent(name: string, payload: unknown): void {
  for (const listener of listeners.get(name) ?? []) listener({ payload });
}

/** How many subscribers an event currently has (to check cleanup on unmount). */
export function listenerCount(name: string): number {
  return listeners.get(name)?.size ?? 0;
}

/** Calls made to one command, in order. */
export function callsTo(command: string): unknown[][] {
  return invokeMock.mock.calls.filter(([name]) => name === command).map(([, ...rest]) => rest);
}

/**
 * A rejected promise shaped like a backend failure. Tauri rejects `invoke` with the serialised
 * `LunaError` (a plain object, not an `Error`), which is exactly what the code under test must handle.
 */
export function backendError(code: string, message: string): Promise<never> {
  // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- see above
  return Promise.reject({ code, message });
}
