import { invoke, isTauri } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { IpcError, ipc, isLunaError, toLunaError } from "./ipc";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const isTauriMock = vi.mocked(isTauri);

beforeEach(() => {
  vi.resetAllMocks();
  isTauriMock.mockReturnValue(true);
});

describe("isLunaError / toLunaError", () => {
  it("recognises the backend error shape", () => {
    expect(isLunaError({ code: "NOT_FOUND", message: "x" })).toBe(true);
    expect(isLunaError({ code: "NOT_FOUND" })).toBe(false);
    expect(isLunaError("boom")).toBe(false);
    expect(isLunaError(null)).toBe(false);
  });

  it("passes backend errors through and wraps everything else as INTERNAL", () => {
    const backend = { code: "IO", message: "disk", details: { path: "C:\\a b" } } as const;
    expect(toLunaError(backend)).toBe(backend);
    expect(toLunaError(new Error("oops"))).toEqual({ code: "INTERNAL", message: "oops" });
    expect(toLunaError("plain string")).toEqual({ code: "INTERNAL", message: "plain string" });
  });
});

describe("ipc.appInfo", () => {
  it("calls the app_info command and returns its payload", async () => {
    const info = {
      appVersion: "0.0.0",
      os: "windows",
      arch: "x86_64",
      lunaHome: "C:\\Users\\Zoë\\AppData\\Local\\lunaRun",
      lunaHomeFromEnv: false,
    };
    invokeMock.mockResolvedValue(info);
    await expect(ipc.appInfo()).resolves.toEqual(info);
    expect(invokeMock).toHaveBeenCalledWith("app_info", undefined);
  });

  it("turns a backend rejection into an IpcError with the typed error", async () => {
    invokeMock.mockRejectedValue({ code: "PERMISSION_DENIED", message: "denied" });
    const error = await ipc.appInfo().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(IpcError);
    expect((error as IpcError).error.code).toBe("PERMISSION_DENIED");
    expect((error as IpcError).message).toBe("denied");
  });

  it("explains that there is no backend when run outside the desktop app", async () => {
    isTauriMock.mockReturnValue(false);
    const error = await ipc.appInfo().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(IpcError);
    expect((error as IpcError).error.code).toBe("INTERNAL");
    expect((error as IpcError).message).toMatch(/desktop app/);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
