import { describe, expect, it } from "vitest";
import { findProblems, parseCargoWorkspaceVersion } from "./check-versions.mjs";

const same = (version) => ({
  "package.json": version,
  "src-tauri/tauri.conf.json": version,
  "Cargo.toml": version,
});

describe("parseCargoWorkspaceVersion", () => {
  it("reads the version under [workspace.package]", () => {
    const toml = `[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "1.2.3"\nedition = "2024"\n`;
    expect(parseCargoWorkspaceVersion(toml)).toBe("1.2.3");
  });

  it("ignores versions in other sections and returns null when absent", () => {
    const toml = `[workspace]\nmembers = []\n\n[profile.release]\nlto = true\n\n[dependencies]\nfoo = { version = "9" }\n`;
    expect(parseCargoWorkspaceVersion(toml)).toBeNull();
  });
});

describe("findProblems", () => {
  it("accepts matching versions with and without a tag", () => {
    expect(findProblems(same("0.1.0"))).toEqual([]);
    expect(findProblems(same("0.1.0"), "v0.1.0")).toEqual([]);
    expect(findProblems(same("0.2.0-beta.1"), "v0.2.0-beta.1")).toEqual([]);
  });

  it("reports files that disagree, naming each one", () => {
    const problems = findProblems({ ...same("0.1.0"), "Cargo.toml": "0.1.1" });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toMatch(/differ/);
    expect(problems[0]).toMatch(/Cargo\.toml=0\.1\.1/);
    expect(problems[0]).toMatch(/package\.json=0\.1\.0/);
  });

  it("reports a tag that does not match the files", () => {
    const problems = findProblems(same("0.1.0"), "v0.2.0");
    expect(problems).toHaveLength(3);
    expect(problems[0]).toMatch(/package\.json is 0\.1\.0 but the tag is v0\.2\.0/);
  });

  it("rejects malformed tags", () => {
    for (const tag of ["0.1.0", "v1.2", "release-1", "v1.2.3.4"]) {
      expect(findProblems(same("1.2.3"), tag).join()).toMatch(/not of the form/);
    }
  });

  it("reports a missing version", () => {
    const problems = findProblems({ ...same("0.1.0"), "Cargo.toml": null });
    expect(problems).toContain("Cargo.toml: no version found");
  });
});
