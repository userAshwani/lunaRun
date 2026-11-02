import { describe, expect, it } from "vitest";
import { generate } from "./generate.mjs";
import { buildPhp } from "./php.mjs";
import {
  buildStaticPhp,
  completePlatforms,
  newestStaticPerLine,
  staticPhpFiles,
} from "./phpstatic.mjs";
import { fakeSource, json } from "./test-support.mjs";

const listing = () => json("static-php-listing.json");
const BASE = "https://dl.static-php.dev/static-php-cli/common";

describe("static php listing", () => {
  it("keeps only cli and fpm archives for macOS and Linux, grouped by version", () => {
    const grouped = staticPhpFiles(listing());
    // The fixture also has micro builds, a README and a directory: none of them are PHP downloads.
    expect([...grouped.keys()].sort()).toEqual([
      "8.0.30",
      "8.1.34",
      "8.2.32",
      "8.3.31",
      "8.3.32",
      "8.4.23",
      "8.5.8",
    ]);
    const keys = [...grouped.get("8.3.32").keys()].sort();
    expect(keys).toHaveLength(12 - 4); // 4 platforms x (cli, fpm), no micro
    expect(keys.some((k) => k.includes("micro"))).toBe(false);
    expect(grouped.get("8.3.32").get("cli|linux|x86_64")).toEqual({
      name: "php-8.3.32-cli-linux-x86_64.tar.gz",
      builtAt: expect.stringMatching(/^\d{4}-\d{2}-\d{2}$/),
    });
  });

  it("takes the newest version of each line, and only versions with a complete CLI and FPM pair", () => {
    expect(newestStaticPerLine(staticPhpFiles(listing()))).toEqual([
      "8.5.8",
      "8.4.23",
      "8.3.32",
      "8.2.32",
      "8.1.34",
      "8.0.30",
    ]);

    // A newer build that is missing its FPM half is not an installable PHP.
    const half = [
      ...listing(),
      {
        name: "php-8.3.40-cli-linux-x86_64.tar.gz",
        is_dir: false,
        last_modified: "2026-10-01 00:00:00",
      },
    ];
    const grouped = staticPhpFiles(half);
    expect(completePlatforms(grouped.get("8.3.40"))).toEqual([]);
    expect(newestStaticPerLine(grouped)).toContain("8.3.32");
    expect(newestStaticPerLine(grouped)).not.toContain("8.3.40");
  });
});

describe("static php builds", () => {
  it("lists the CLI and the FPM archive of every platform as two variants of one version", async () => {
    const versions = await buildStaticPhp(fakeSource());
    expect(versions.map((v) => v.version)).toEqual([
      "8.5.8",
      "8.4.23",
      "8.3.32",
      "8.2.32",
      "8.1.34",
      "8.0.30",
    ]);

    const v83 = versions.find((v) => v.version === "8.3.32");
    expect(v83.artifacts.map((a) => `${a.os}/${a.arch}/${a.variant}`)).toEqual([
      "macos/aarch64/static-cli",
      "macos/aarch64/static-fpm",
      "macos/x86_64/static-cli",
      "macos/x86_64/static-fpm",
      "linux/x86_64/static-cli",
      "linux/x86_64/static-fpm",
      "linux/aarch64/static-cli",
      "linux/aarch64/static-fpm",
    ]);
    const cli = v83.artifacts[0];
    expect(cli).toMatchObject({
      format: "tar.gz",
      url: `${BASE}/php-8.3.32-cli-macos-aarch64.tar.gz`,
      stripComponents: 0, // each archive holds one file, `php` or `php-fpm`
    });
    expect(cli.sha256).toMatch(/^[0-9a-f]{64}$/);
    // The extension list is the one the build really has, and says so for both halves.
    expect(cli.extensions).toEqual(json("static-php-extensions.json"));
    expect(cli.extensions).toContain("pdo_sqlite");
    expect(cli.extensions).not.toContain("intl");
    expect(v83.artifacts[1].extensions).toEqual(cli.extensions);
    expect(v83.releasedAt).toBe(
      v83.artifacts
        .map((a) => a.builtAt)
        .sort()
        .at(-1),
    );
  });

  it("dates a version by its newest file", async () => {
    const mixed = listing().map((e) =>
      e.name === "php-8.3.32-fpm-macos-x86_64.tar.gz"
        ? { ...e, last_modified: "2031-05-06 07:08:09" }
        : e,
    );
    const versions = await buildStaticPhp({ ...fakeSource(), staticPhpListing: async () => mixed });
    expect(versions.find((v) => v.version === "8.3.32").releasedAt).toBe("2031-05-06");
  });

  it("offers a platform only when both of its archives exist", async () => {
    const trimmed = listing().filter((e) => e.name !== "php-8.5.8-fpm-linux-aarch64.tar.gz");
    const versions = await buildStaticPhp({
      ...fakeSource(),
      staticPhpListing: async () => trimmed,
    });
    const v85 = versions.find((v) => v.version === "8.5.8");
    expect(v85.artifacts.some((a) => a.os === "linux" && a.arch === "aarch64")).toBe(false);
    expect(v85.artifacts).toHaveLength(6);
  });

  it("downloads a file again only when upstream rebuilt it", async () => {
    const first = await buildStaticPhp(fakeSource());
    const previous = { versions: first };

    const never = fakeSource({
      hash: async () => {
        throw new Error("must not download");
      },
    });
    expect(await buildStaticPhp(never, { previous })).toEqual(first);

    // One file is rebuilt under the same name: its last-modified date moves.
    const rebuilt = listing().map((e) =>
      e.name === "php-8.3.32-fpm-linux-x86_64.tar.gz"
        ? { ...e, last_modified: "2030-01-02 03:04:05" }
        : e,
    );
    const downloads = [];
    const base = fakeSource();
    const spy = {
      ...base,
      staticPhpListing: async () => rebuilt,
      hash: async (url) => {
        downloads.push(url);
        return base.hash(url);
      },
    };
    const again = await buildStaticPhp(spy, { previous });
    expect(downloads).toEqual([`${BASE}/php-8.3.32-fpm-linux-x86_64.tar.gz`]);
    const changed = again
      .find((v) => v.version === "8.3.32")
      .artifacts.find((a) => a.url === downloads[0]);
    expect(changed.builtAt).toBe("2030-01-02");
  });

  it("fails loudly with no extension list or no complete build", async () => {
    const noExtensions = { ...fakeSource(), staticPhpExtensions: async () => [] };
    await expect(buildStaticPhp(noExtensions)).rejects.toThrow(/lists no extension/);
    const none = { ...fakeSource(), staticPhpListing: async () => [] };
    await expect(buildStaticPhp(none)).rejects.toThrow(/no complete PHP build/);
  });
});

describe("php: Windows and static builds together", () => {
  it("keeps separate versions when the numbers differ and merges the downloads when they match", async () => {
    const apart = await buildPhp(fakeSource());
    // Windows has 8.3.36 and the static build is 8.3.32: two entries on the same line.
    expect(
      apart.versions.filter((v) => v.version.startsWith("8.3.")).map((v) => v.version),
    ).toEqual(["8.3.36", "8.3.32"]);
    expect(apart.versions.find((v) => v.version === "8.3.36").artifacts.map((a) => a.os)).toEqual([
      "windows",
    ]);

    // The day both ship 8.3.32, it is one entry with every download.
    const releases = json("php-releases.json");
    releases["8.3"].version = "8.3.32-1";
    const together = await buildPhp({ ...fakeSource(), phpReleases: async () => releases });
    const merged = together.versions.filter((v) => v.version === "8.3.32");
    expect(merged).toHaveLength(1);
    expect(merged[0].artifacts.map((a) => a.os)).toEqual([
      "windows",
      ...Array(8).fill("macos").slice(0, 4),
      ...Array(4).fill("linux"),
    ]);
    expect(merged[0].releasedAt).toBe("2026-10-26"); // the Windows date wins
  });

  it("produces a registry that passes the generator's own rules, newest first", async () => {
    const { registry, errors } = await generate(fakeSource());
    expect(errors).toEqual([]);
    const versions = registry.runtimes.php.versions.map((v) => v.version);
    expect(versions).toEqual(
      [...versions].sort((a, b) => {
        const [x, y] = [a, b].map((v) => v.split(".").map(Number));
        return y[0] - x[0] || y[1] - x[1] || y[2] - x[2];
      }),
    );
  });
});
