import { describe, expect, it } from "vitest";
import type { AppInfo } from "@/bindings/AppInfo";
import { buildReport } from "./report";

const info: AppInfo = {
  appVersion: "0.1.0",
  os: "macos",
  arch: "aarch64",
  lunaHome: "/Users/zoe/Library/Application Support/lunaRun",
  lunaHomeFromEnv: false,
};

describe("buildReport", () => {
  it("lists version, platform and data folder as plain text", () => {
    expect(buildReport(info)).toBe(
      [
        "lunaRun diagnostics",
        "Version: 0.1.0",
        "OS: macos",
        "Architecture: aarch64",
        "Data folder: /Users/zoe/Library/Application Support/lunaRun",
      ].join("\n"),
    );
  });

  it("marks a data folder that came from LUNA_HOME", () => {
    expect(buildReport({ ...info, lunaHomeFromEnv: true })).toContain("(from LUNA_HOME)");
  });
});
