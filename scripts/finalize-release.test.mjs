import { describe, expect, it } from "vitest";
import { INSTALLERS, finalizeRelease, planRenames } from "./finalize-release.mjs";

/** The exact asset names of the real v0.0.1 draft release. */
const V001 = [
  "lunaRun-0.0.1-1.x86_64.rpm",
  "lunaRun_0.0.1_amd64.AppImage",
  "lunaRun_0.0.1_amd64.deb",
  "lunaRun_0.0.1_universal.app.tar.gz",
  "lunaRun_0.0.1_universal.dmg",
  "lunaRun_0.0.1_x64-setup.exe",
  "lunaRun_0.0.1_x64_en-US.msi",
];

describe("planRenames", () => {
  it("maps the real v0.0.1 assets to the stable names", () => {
    const { renames, problems } = planRenames(V001);
    expect(problems).toEqual([]);
    expect(Object.fromEntries(renames.map((r) => [r.from, r.to]))).toEqual({
      "lunaRun-0.0.1-1.x86_64.rpm": "lunaRun-linux-x64.rpm",
      "lunaRun_0.0.1_amd64.AppImage": "lunaRun-linux-x64.AppImage",
      "lunaRun_0.0.1_amd64.deb": "lunaRun-linux-x64.deb",
      "lunaRun_0.0.1_universal.app.tar.gz": "lunaRun-macos-universal.app.tar.gz",
      "lunaRun_0.0.1_universal.dmg": "lunaRun-macos-universal.dmg",
      "lunaRun_0.0.1_x64-setup.exe": "lunaRun-windows-x64-setup.exe",
      "lunaRun_0.0.1_x64_en-US.msi": "lunaRun-windows-x64.msi",
    });
  });

  it("works for any version number", () => {
    const names = V001.map((name) => name.replace(/0\.0\.1/g, "12.34.56-beta.2"));
    expect(planRenames(names).problems).toEqual([]);
  });

  it("is idempotent: already-stable names need no renames", () => {
    const stable = INSTALLERS.map((installer) => installer.stable);
    expect(planRenames(stable)).toEqual({ renames: [], problems: [] });
  });

  it("finishes a half-renamed release", () => {
    const names = V001.filter((n) => !n.endsWith(".dmg")).concat("lunaRun-macos-universal.dmg");
    const { renames, problems } = planRenames(names);
    expect(problems).toEqual([]);
    expect(renames).toHaveLength(INSTALLERS.length - 1);
  });

  it("reports every missing installer, which blocks publishing", () => {
    const { problems } = planRenames(
      V001.filter((n) => !n.endsWith(".dmg") && !n.endsWith(".msi")),
    );
    expect(problems).toEqual([
      "missing: no asset matches lunaRun-windows-x64.msi",
      "missing: no asset matches lunaRun-macos-universal.dmg",
    ]);
  });

  it("refuses ambiguous and unexpected assets", () => {
    const ambiguous = planRenames([...V001, "lunaRun_0.0.1_x86_en-US.msi"]).problems.join("\n");
    expect(ambiguous).toMatch(/ambiguous: .* all match lunaRun-windows-x64\.msi/);
    expect(planRenames([...V001, "notes.txt"]).problems).toEqual(["unexpected asset notes.txt"]);
  });
});

describe("finalizeRelease", () => {
  const asset = (name, id) => ({
    name,
    apiUrl: `https://api.github.com/repos/o/r/releases/assets/${id}`,
  });

  /** A fake `gh` that serves one release and records every call. */
  const fakeGh = (names, { isDraft = true, isPrerelease = false, failOn } = {}) => {
    const calls = [];
    const run = (command, args) => {
      calls.push([command, ...args]);
      if (failOn && args.join(" ").includes(failOn))
        return { status: 1, stdout: "", stderr: "boom" };
      if (args[0] === "release" && args[1] === "view") {
        const assets = names.map((name, index) => asset(name, 100 + index));
        return { status: 0, stdout: JSON.stringify({ assets, isDraft, isPrerelease }), stderr: "" };
      }
      return { status: 0, stdout: "", stderr: "" };
    };
    return {
      run,
      calls,
      mutations: () => calls.filter((c) => c[1] !== "release" || c[2] !== "view"),
    };
  };

  it("renames every installer, then publishes the draft", () => {
    const gh = fakeGh(V001);
    const result = finalizeRelease({ tag: "v0.0.1", repo: "o/r", publish: true, run: gh.run });
    expect(result.renames).toHaveLength(7);
    const patches = gh.calls.filter((c) => c.includes("PATCH"));
    expect(patches).toHaveLength(7);
    // Renames follow the INSTALLERS order, so the Windows installer (index 5 in V001, id 105) goes first.
    expect(patches[0]).toEqual([
      "gh",
      "api",
      "--method",
      "PATCH",
      "repos/o/r/releases/assets/105",
      "-f",
      "name=lunaRun-windows-x64-setup.exe",
    ]);
    expect(gh.calls.at(-1)).toEqual([
      "gh",
      "release",
      "edit",
      "v0.0.1",
      "--repo",
      "o/r",
      "--draft=false",
    ]);
  });

  it("does not publish unless asked, and not when already public", () => {
    const withoutFlag = fakeGh(V001);
    finalizeRelease({ tag: "v1", repo: "o/r", run: withoutFlag.run });
    expect(withoutFlag.calls.some((c) => c[2] === "edit")).toBe(false);

    const alreadyPublic = fakeGh(V001, { isDraft: false });
    finalizeRelease({ tag: "v1", repo: "o/r", publish: true, run: alreadyPublic.run });
    expect(alreadyPublic.calls.some((c) => c[2] === "edit")).toBe(false);
  });

  it("changes nothing on --dry-run", () => {
    const gh = fakeGh(V001);
    const result = finalizeRelease({
      tag: "v1",
      repo: "o/r",
      publish: true,
      dryRun: true,
      run: gh.run,
    });
    expect(result).toMatchObject({ dryRun: true, willPublish: true });
    expect(gh.mutations()).toEqual([]);
  });

  it("neither renames nor publishes an incomplete release", () => {
    const gh = fakeGh(V001.slice(0, 5));
    expect(() => finalizeRelease({ tag: "v1", repo: "o/r", publish: true, run: gh.run })).toThrow(
      /not complete, nothing was changed/,
    );
    expect(gh.mutations()).toEqual([]);
  });

  it("does not publish if a rename fails", () => {
    const gh = fakeGh(V001, { failOn: "name=lunaRun-macos-universal.dmg" });
    expect(() => finalizeRelease({ tag: "v1", repo: "o/r", publish: true, run: gh.run })).toThrow(
      /boom/,
    );
    expect(gh.calls.some((c) => c[2] === "edit")).toBe(false);
  });

  it("reports a missing gh binary clearly", () => {
    const run = () => ({ error: new Error("spawn gh ENOENT"), status: null });
    expect(() => finalizeRelease({ tag: "v1", repo: "o/r", run })).toThrow(/cannot run gh/);
  });
});
