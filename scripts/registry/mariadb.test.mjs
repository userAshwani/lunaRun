import { describe, expect, it } from "vitest";
import { buildMariadb } from "./mariadb.mjs";
import { fakeSource, json } from "./test-support.mjs";

/** A release as the API describes it, with just the files a test needs. */
const release = (date, names) => ({
  date_of_release: date,
  files: names.map((file_name) => ({ file_name, checksum: { sha256sum: "ab".repeat(32) } })),
});

/** A source with one long-term-support line `9.9` whose releases the test chooses. */
const oneLine = (releases) => ({
  ...fakeSource(),
  mariadbMajors: async () => ({
    major_releases: [
      {
        release_id: "9.9",
        release_status: "Stable",
        release_support_type: "Long Term Support",
        release_eol_date: null,
      },
    ],
  }),
  mariadbLine: async () => ({ releases }),
});

describe("mariadb", () => {
  it("offers only stable long-term-support lines, with the end-of-life dates MariaDB publishes", async () => {
    const mariadb = await buildMariadb(fakeSource());
    // 13.x is rolling and 13.2 is a preview: neither is offered.
    expect(Object.keys(mariadb.lines).sort()).toEqual(["10.11", "10.6", "11.4", "11.8", "12.3"]);
    expect(mariadb.lines["10.6"]).toEqual({ eol: "2026-07-06" });
    expect(mariadb.lines["12.3"]).toEqual({ eol: "2029-06-12" });
  });

  it("takes the newest release of each line, newest line first", async () => {
    const mariadb = await buildMariadb(fakeSource());
    expect(mariadb.versions.map((v) => v.version)).toEqual([
      "12.3.3",
      "11.8.9",
      "11.4.13",
      "10.11.19",
      "10.6.28",
    ]);
  });

  it("copies the published SHA-256 and uses archive.mariadb.org with Windows and Linux only", async () => {
    const mariadb = await buildMariadb(fakeSource());
    const v118 = mariadb.versions.find((v) => v.version === "11.8.9");
    expect(v118.releasedAt).toBe("2026-08-22");
    expect(v118.artifacts).toEqual([
      {
        os: "windows",
        arch: "x86_64",
        format: "zip",
        url: "https://archive.mariadb.org/mariadb-11.8.9/winx64-packages/mariadb-11.8.9-winx64.zip",
        sha256: "830c46727d9278eae212ae3eca44eeb9e71b2a68704e95f344a64fba7b1963f5",
        size:
          "https://archive.mariadb.org/mariadb-11.8.9/winx64-packages/mariadb-11.8.9-winx64.zip"
            .length * 1000,
        stripComponents: 1,
        requires: ["vcredist-14-x64"],
      },
      {
        os: "linux",
        arch: "x86_64",
        format: "tar.gz",
        url: "https://archive.mariadb.org/mariadb-11.8.9/bintar-linux-systemd-x86_64/mariadb-11.8.9-linux-systemd-x86_64.tar.gz",
        sha256: "32f8871a2aa38b36aa418b7f4abc6fc0c6ef1ba5a45798cacb2afa52c8a6113b",
        size: expect.any(Number),
        stripComponents: 1,
      },
    ]);
    // The .msi and the debug-symbols zip sit in the same release and must never be picked.
    const all = mariadb.versions.flatMap((v) => v.artifacts.map((a) => a.url)).join("\n");
    expect(all).not.toMatch(/msi|debugsymbols/);
    expect(mariadb.versions.flatMap((v) => v.artifacts).some((a) => a.os === "macos")).toBe(false);
  });

  it("skips a newer release that has no archive and uses a partial one if that is what exists", async () => {
    const mariadb = await buildMariadb(
      oneLine({
        "9.9.3": release("2026-03-01", ["mariadb-9.9.3-winx64.msi"]), // only an installer
        "9.9.2": release("2026-02-01", ["mariadb-9.9.2-linux-systemd-x86_64.tar.gz"]), // no Windows zip
        "9.9.1": release("2026-01-01", ["mariadb-9.9.1-winx64.zip"]),
        "9.9.0-preview": release("2025-12-01", ["mariadb-9.9.0-preview-winx64.zip"]),
      }),
    );
    expect(mariadb.versions.map((v) => v.version)).toEqual(["9.9.2"]);
    expect(mariadb.versions[0].artifacts.map((a) => a.os)).toEqual(["linux"]);
  });

  it("never offers a release candidate or preview, whatever its position in the list", async () => {
    const mariadb = await buildMariadb(
      oneLine({
        "9.9.5-rc": release("2026-05-01", ["mariadb-9.9.5-rc-winx64.zip"]),
        "9.9.1": release("2026-01-01", ["mariadb-9.9.1-winx64.zip"]),
      }),
    );
    expect(mariadb.versions.map((v) => v.version)).toEqual(["9.9.1"]);
  });

  it("compares release numbers as numbers", async () => {
    const mariadb = await buildMariadb(
      oneLine({
        "9.9.9": release("2026-01-01", ["mariadb-9.9.9-winx64.zip"]),
        "9.9.10": release("2026-02-01", ["mariadb-9.9.10-winx64.zip"]),
      }),
    );
    expect(mariadb.versions[0].version).toBe("9.9.10");
  });

  it("fails loudly on a bad checksum, a file that is not on the archive, or a line with nothing", async () => {
    const bad = oneLine({ "9.9.1": release("2026-01-01", ["mariadb-9.9.1-winx64.zip"]) });
    bad.mariadbLine = async () => ({
      releases: {
        "9.9.1": {
          date_of_release: "2026-01-01",
          files: [{ file_name: "mariadb-9.9.1-winx64.zip", checksum: { sha256sum: null } }],
        },
      },
    });
    await expect(buildMariadb(bad)).rejects.toThrow(/not a SHA-256/);

    await expect(buildMariadb(fakeSource({ size: () => null }))).rejects.toThrow(
      /listed by the API but not on archive\.mariadb\.org/,
    );
    await expect(
      buildMariadb(oneLine({ "9.9.1": release("2026-01-01", ["mariadb-9.9.1.tar.gz"]) })),
    ).rejects.toThrow(/no release has a Windows or Linux archive/);

    const none = {
      ...fakeSource(),
      mariadbMajors: async () => ({
        major_releases: json("mariadb-majors.json").major_releases.filter(
          (m) => m.release_status !== "Stable",
        ),
      }),
    };
    await expect(buildMariadb(none)).rejects.toThrow(/no stable long-term-support line/);
  });
});
