import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  bump,
  compareVersions,
  parseVersion,
  promoteChangelog,
  setCargoWorkspaceVersion,
  setJsonVersion,
} from "./bump-version.mjs";

describe("compareVersions", () => {
  it("orders by major, minor, patch", () => {
    expect(compareVersions("1.0.0", "0.9.9")).toBeGreaterThan(0);
    expect(compareVersions("0.2.0", "0.10.0")).toBeLessThan(0);
    expect(compareVersions("1.2.3", "1.2.3")).toBe(0);
  });

  it("puts a release above its prereleases and orders prerelease identifiers", () => {
    const ascending = [
      "1.0.0-alpha",
      "1.0.0-alpha.1",
      "1.0.0-alpha.beta",
      "1.0.0-beta.2",
      "1.0.0-beta.11",
      "1.0.0-rc.1",
      "1.0.0",
    ];
    for (let i = 1; i < ascending.length; i++) {
      expect(
        compareVersions(ascending[i - 1], ascending[i]),
        `${ascending[i - 1]} < ${ascending[i]}`,
      ).toBeLessThan(0);
      expect(compareVersions(ascending[i], ascending[i - 1])).toBeGreaterThan(0);
    }
  });

  it("rejects things that are not versions", () => {
    expect(() => compareVersions("v1.0.0", "1.0.0")).toThrow(/cannot compare/);
  });
});

describe("parseVersion", () => {
  it("accepts plain and prerelease versions only", () => {
    expect(parseVersion("0.1.0")).toEqual({ core: [0, 1, 0], pre: [] });
    expect(parseVersion("0.1.0-beta.1")).toEqual({ core: [0, 1, 0], pre: ["beta", "1"] });
    for (const bad of ["v0.1.0", "1.2", "1.2.3.4", "", "latest", "1.2.3 "]) {
      expect(parseVersion(bad), JSON.stringify(bad)).toBeNull();
    }
  });
});

describe("setJsonVersion", () => {
  it("changes only the version and keeps formatting", () => {
    const text = `{\n  "name": "x",\n  "version": "0.0.0",\n  "scripts": {\n    "a": "b"\n  }\n}\n`;
    expect(setJsonVersion(text, "0.1.0", "package.json")).toBe(text.replace("0.0.0", "0.1.0"));
  });

  it("keeps Windows line endings", () => {
    const text = `{\r\n  "version": "0.0.0"\r\n}\r\n`;
    expect(setJsonVersion(text, "1.0.0", "x")).toBe(`{\r\n  "version": "1.0.0"\r\n}\r\n`);
  });

  it("fails clearly when there is no version", () => {
    expect(() => setJsonVersion(`{ "name": "x" }`, "1.0.0", "x.json")).toThrow(/x\.json/);
  });
});

describe("setCargoWorkspaceVersion", () => {
  const toml = [
    "[workspace]",
    'members = ["a"]',
    "",
    "[workspace.package]",
    'version = "0.0.0"',
    'edition = "2024"',
    "",
    "[workspace.dependencies]",
    'foo = { version = "9.9.9" }',
    "",
  ].join("\n");

  it("touches only [workspace.package]", () => {
    const next = setCargoWorkspaceVersion(toml, "0.3.0");
    expect(next).toBe(toml.replace('version = "0.0.0"', 'version = "0.3.0"'));
    expect(next).toContain('foo = { version = "9.9.9" }');
  });

  it("fails when the section has no version", () => {
    expect(() =>
      setCargoWorkspaceVersion('[workspace.package]\nedition = "2024"\n', "1.0.0"),
    ).toThrow(/workspace\.package/);
  });
});

describe("promoteChangelog", () => {
  const log = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "",
    "### Added",
    "- New thing",
    "",
    "## [0.0.9] - 2026-01-01",
    "",
    "### Fixed",
    "- Old thing",
    "",
  ].join("\n");

  it("dates the unreleased notes and leaves a fresh empty section", () => {
    expect(promoteChangelog(log, "0.1.0", "2026-10-30")).toBe(
      [
        "# Changelog",
        "",
        "## [Unreleased]",
        "",
        "## [0.1.0] - 2026-10-30",
        "",
        "### Added",
        "- New thing",
        "",
        "## [0.0.9] - 2026-01-01",
        "",
        "### Fixed",
        "- Old thing",
        "",
      ].join("\n"),
    );
  });

  it("works when Unreleased is the only section and keeps CRLF", () => {
    const text = "# Changelog\r\n\r\n## [Unreleased]\r\n\r\n- A\r\n";
    expect(promoteChangelog(text, "1.0.0", "2026-10-30")).toBe(
      "# Changelog\r\n\r\n## [Unreleased]\r\n\r\n## [1.0.0] - 2026-10-30\r\n\r\n- A\r\n",
    );
  });

  it("refuses an empty Unreleased section, a missing one, and a duplicate version", () => {
    expect(() =>
      promoteChangelog("## [Unreleased]\n\n## [0.0.9] - x\n- a\n", "0.1.0", "d"),
    ).toThrow(/empty/);
    expect(() => promoteChangelog("# Changelog\n", "0.1.0", "d")).toThrow(/Unreleased/);
    expect(() => promoteChangelog(log, "0.0.9", "d")).toThrow(/already exists/);
  });
});

describe("bump (on a throwaway checkout)", () => {
  let root;

  const read = (file) => readFileSync(path.join(root, file), "utf8");
  const files = () => ({
    pkg: read("package.json"),
    tauri: read("src-tauri/tauri.conf.json"),
    cargo: read("Cargo.toml"),
    log: read("CHANGELOG.md"),
    lock: read("Cargo.lock"),
  });

  beforeEach(() => {
    root = mkdtempSync(path.join(tmpdir(), "lüna bump "));
    mkdirSync(path.join(root, "src-tauri"));
    writeFileSync(path.join(root, "package.json"), `{\n  "name": "x",\n  "version": "0.0.0"\n}\n`);
    writeFileSync(
      path.join(root, "src-tauri/tauri.conf.json"),
      `{\n  "productName": "x",\n  "version": "0.0.0"\n}\n`,
    );
    writeFileSync(path.join(root, "Cargo.toml"), `[workspace.package]\nversion = "0.0.0"\n`);
    writeFileSync(
      path.join(root, "CHANGELOG.md"),
      `# Changelog\n\n## [Unreleased]\n\n- Something\n`,
    );
    writeFileSync(path.join(root, "Cargo.lock"), "lock v0.0.0\n");
  });

  afterEach(() => {
    rmSync(root, { recursive: true, force: true });
  });

  /** Pretends to be `cargo update`, and rewrites the lock file like the real one would. */
  const fakeCargo =
    (status = 0, stderr = "") =>
    (command, args, options) => {
      expect([command, ...args]).toEqual(["cargo", "update", "--workspace", "--offline"]);
      expect(options.cwd).toBe(root);
      writeFileSync(path.join(root, "Cargo.lock"), "lock v0.1.0\n");
      return { status, stderr };
    };

  it("updates all four files and the lock, and the result passes the consistency check", () => {
    const summary = bump({ root, version: "0.1.0", date: "2026-10-30", run: fakeCargo() });
    expect(summary).toMatchObject({ current: "0.0.0", version: "0.1.0", dryRun: false });
    expect(summary.files).toEqual([
      "package.json",
      "src-tauri/tauri.conf.json",
      "Cargo.toml",
      "CHANGELOG.md",
      "Cargo.lock",
    ]);

    const after = files();
    expect(after.pkg).toContain('"version": "0.1.0"');
    expect(after.tauri).toContain('"version": "0.1.0"');
    expect(after.cargo).toContain('version = "0.1.0"');
    expect(after.log).toContain("## [Unreleased]\n\n## [0.1.0] - 2026-10-30\n\n- Something");
    expect(after.lock).toBe("lock v0.1.0\n");
  });

  it("changes nothing on --dry-run", () => {
    const before = files();
    const summary = bump({ root, version: "0.1.0", date: "d", dryRun: true, run: fakeCargo() });
    expect(summary.dryRun).toBe(true);
    expect(files()).toEqual(before);
  });

  it("can skip the lock refresh", () => {
    const summary = bump({ root, version: "0.1.0", date: "d", updateLock: false });
    expect(summary.files).not.toContain("Cargo.lock");
    expect(read("Cargo.lock")).toBe("lock v0.0.0\n");
  });

  it("rejects an invalid, equal or lower version without touching anything", () => {
    const before = files();
    expect(() => bump({ root, version: "v0.1.0", date: "d", updateLock: false })).toThrow(
      /not a valid version/,
    );
    expect(() => bump({ root, version: "0.0.0", date: "d", updateLock: false })).toThrow(
      /not greater/,
    );
    expect(files()).toEqual(before);
  });

  it("allows a lower version only with --force", () => {
    bump({ root, version: "0.1.0", date: "d", updateLock: false });
    writeFileSync(path.join(root, "CHANGELOG.md"), `## [Unreleased]\n\n- Again\n`);
    expect(() => bump({ root, version: "0.0.5", date: "d", updateLock: false })).toThrow(
      /not greater/,
    );
    bump({ root, version: "0.0.5", date: "d", updateLock: false, force: true });
    expect(read("package.json")).toContain('"version": "0.0.5"');
  });

  it("changes nothing when the changelog cannot be promoted", () => {
    writeFileSync(path.join(root, "CHANGELOG.md"), `# Changelog\n\n## [Unreleased]\n\n`);
    const before = files();
    expect(() => bump({ root, version: "0.1.0", date: "d", updateLock: false })).toThrow(/empty/);
    expect(files()).toEqual(before);
  });

  it("puts every file back, lock included, if cargo fails", () => {
    const before = files();
    expect(() =>
      bump({ root, version: "0.1.0", date: "d", run: fakeCargo(101, "no network") }),
    ).toThrow(/no network/);
    expect(files()).toEqual(before);
  });

  it("puts every file back if cargo cannot be started", () => {
    const before = files();
    const missing = () => ({ error: new Error("spawn cargo ENOENT"), status: null, stderr: "" });
    expect(() => bump({ root, version: "0.1.0", date: "d", run: missing })).toThrow(/ENOENT/);
    expect(files()).toEqual(before);
  });
});
