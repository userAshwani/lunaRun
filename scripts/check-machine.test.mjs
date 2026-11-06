import { describe, expect, it } from "vitest";
import {
  FIXES,
  compareVersions,
  fixFor,
  parseAheadBehind,
  parsePnpmVersion,
  parseRustVersion,
  pushWasDenied,
  renderReport,
  versionIn,
} from "./check-machine.mjs";

describe("compareVersions", () => {
  it("compares numbers, not text, and treats a missing part as 0", () => {
    expect(compareVersions("1.99.1", "1.99")).toBeGreaterThan(0);
    expect(compareVersions("1.99", "1.99.0")).toBe(0);
    expect(compareVersions("1.9", "1.10")).toBeLessThan(0);
    expect(compareVersions("2.0.0", "1.99.9")).toBeGreaterThan(0);
    expect(compareVersions("1.98.4", "1.99")).toBeLessThan(0);
  });

  it("ignores a pre-release suffix", () => {
    expect(compareVersions("1.99.0-nightly", "1.99.0")).toBe(0);
  });
});

describe("reading versions", () => {
  it("finds the first version in a program's output", () => {
    expect(versionIn("git version 2.51.0.windows.1")).toBe("2.51.0");
    expect(versionIn("rustc 1.99.1 (58cf8f9c3 2026-10-13)")).toBe("1.99.1");
    expect(versionIn("12.16.1\n")).toBe("12.16.1");
    expect(versionIn("no digits here")).toBeNull();
    expect(versionIn(undefined)).toBeNull();
  });

  it("reads rust-version from [workspace.package] only", () => {
    const toml = `[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.0.2"\nrust-version = "1.99"\n\n[dependencies]\nrust-version = "1.0"\n`;
    expect(parseRustVersion(toml)).toBe("1.99");
    expect(parseRustVersion(`[workspace]\nmembers = []\n`)).toBeNull();
  });

  it("reads the pinned pnpm from package.json", () => {
    expect(parsePnpmVersion(`{"packageManager": "pnpm@12.16.1"}`)).toBe("12.16.1");
    expect(parsePnpmVersion(`{"packageManager": "pnpm@12.16.1+sha512.abc"}`)).toBe("12.16.1");
    expect(parsePnpmVersion(`{"packageManager": "yarn@4.0.0"}`)).toBeNull();
    expect(parsePnpmVersion(`{}`)).toBeNull();
    expect(parsePnpmVersion("not json")).toBeNull();
  });

  it("reads ahead and behind from git", () => {
    expect(parseAheadBehind("2\t5\n")).toEqual({ ahead: 2, behind: 5 });
    expect(parseAheadBehind("0 0")).toEqual({ ahead: 0, behind: 0 });
    expect(parseAheadBehind("fatal: bad revision")).toBeNull();
  });
});

describe("pushWasDenied", () => {
  it("recognises a refused login but not a network failure or a stale branch", () => {
    expect(pushWasDenied("fatal: Authentication failed for 'https://github.com/x/y.git/'")).toBe(
      true,
    );
    expect(
      pushWasDenied(
        "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
      ),
    ).toBe(true);
    expect(pushWasDenied("remote: Permission to userAshwani/lunaRun.git denied to someone.")).toBe(
      true,
    );
    expect(
      pushWasDenied("fatal: unable to access 'https://github.com/': Could not resolve host"),
    ).toBe(false);
    expect(pushWasDenied(" ! [rejected]        HEAD -> main (non-fast-forward)")).toBe(false);
    expect(pushWasDenied("Everything up-to-date")).toBe(false);
  });
});

describe("fixFor", () => {
  it("gives each system its own command and falls back to the shared one", () => {
    expect(fixFor("git", "win32")).toContain("winget");
    expect(fixFor("git", "darwin")).toContain("xcode-select");
    expect(fixFor("git", "linux")).toContain("apt");
    expect(fixFor("deps", "linux")).toBe("pnpm install --frozen-lockfile");
  });

  it("puts the pinned pnpm version into the command", () => {
    expect(fixFor("pnpm", "linux", "12.16.1")).toBe(
      "corepack enable && corepack prepare pnpm@12.16.1 --activate   (or: npm install -g pnpm@12.16.1)",
    );
  });

  it("points at the docs for something this system has no fix for", () => {
    expect(fixFor("msvc", "linux")).toContain("tauri.app");
    expect(fixFor("nothing-like-this", "win32")).toContain("tauri.app");
  });

  it("has a fix for every system wherever a tool is needed on all three", () => {
    for (const id of ["git", "node", "rust", "gh"]) {
      for (const platform of ["win32", "darwin", "linux"]) {
        expect(FIXES[id][platform] ?? FIXES[id].all, `${id} on ${platform}`).toBeTruthy();
      }
    }
  });
});

describe("renderReport", () => {
  const ok = { state: "ok", name: "git", detail: "2.51.0", fix: "" };
  const missing = { state: "missing", name: "Rust", detail: "not found", fix: "install rustup" };
  const warn = {
    state: "warn",
    name: "pnpm",
    detail: "11.0.0, the repo pins 12.16.1",
    fix: "fix pnpm",
  };

  it("is ready when nothing required is missing, even with warnings", () => {
    const report = renderReport([ok, warn], "heading");
    expect(report.ready).toBe(true);
    expect(report.text).toContain("READY");
    expect(report.text).not.toContain("NOT READY");
    expect(report.text).toContain("fix: fix pnpm");
  });

  it("is not ready, and names what is missing, with its fix", () => {
    const report = renderReport([ok, missing, { ...missing, name: "pnpm" }], "heading");
    expect(report.ready).toBe(false);
    expect(report.text).toContain("NOT READY: 2 required items missing (Rust, pnpm).");
    expect(report.text).toContain("MISSING  Rust: not found");
    expect(report.text).toContain("fix: install rustup");
    expect(renderReport([missing], "h").text).toContain("1 required item missing");
  });

  it("does not print a fix for something that is fine", () => {
    expect(renderReport([{ ...ok, fix: "should not show" }], "h").text).not.toContain(
      "should not show",
    );
  });
});
