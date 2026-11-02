import { describe, expect, it } from "vitest";
import { generate, liveSource } from "./generate.mjs";
import { compareVersions, parseShasums, registryText, validateRegistry } from "./lib.mjs";
import { buildNode, newestPerMajor } from "./node.mjs";
import { buildPhp, phpVersion } from "./php.mjs";
import { fakeSource, json, text } from "./test-support.mjs";

describe("lib", () => {
  it("compares versions as numbers, not as text", () => {
    expect(compareVersions("8.10.0", "8.9.0")).toBeGreaterThan(0);
    expect(compareVersions("8.3.9", "8.3.14")).toBeLessThan(0);
    expect(compareVersions("8.3", "8.3.0")).toBe(0);
  });

  it("reads SHASUMS256.txt and ignores anything that is not a checksum line", () => {
    const sums = parseShasums(
      `${"A".repeat(64)}  node-v1.zip\r\n${"b".repeat(64)} *node-v2.zip\nnot a checksum line\n${"c".repeat(63)}  short\n`,
    );
    expect([...sums]).toEqual([
      ["node-v1.zip", "a".repeat(64)],
      ["node-v2.zip", "b".repeat(64)],
    ]);
  });

  it("refuses a registry that breaks the rules the app enforces", () => {
    const artifact = {
      os: "linux",
      arch: "x86_64",
      format: "zip",
      url: "https://x.test/a.zip",
      sha256: "0".repeat(64),
      size: 1,
    };
    const registry = (change) => ({
      schema: 1,
      runtimes: {
        php: { versions: [{ version: "8.3.1", artifacts: [{ ...artifact, ...change }] }] },
      },
    });
    expect(validateRegistry(registry({}))).toEqual([]);
    for (const change of [
      { url: "http://x.test/a.zip" },
      { sha256: "abc" },
      { sha256: "g".repeat(64) },
      { size: 0 },
      { size: 1.5 },
    ]) {
      expect(validateRegistry(registry(change)), JSON.stringify(change)).not.toEqual([]);
    }
    const twice = registry({});
    twice.runtimes.php.versions.push(twice.runtimes.php.versions[0]);
    expect(validateRegistry(twice).join()).toContain("listed twice");
    const sameTarget = registry({});
    sameTarget.runtimes.php.versions[0].artifacts.push(artifact);
    expect(validateRegistry(sameTarget).join()).toContain("two downloads");
    const empty = registry({});
    empty.runtimes.php.versions[0].artifacts = [];
    expect(validateRegistry(empty).join()).toContain("no downloads");
    expect(
      validateRegistry({ ...registry({}), runtimes: { "../php": { versions: [] } } }),
    ).not.toEqual([]);
  });
});

describe("php", () => {
  it("drops the Windows packaging revision so the version sorts correctly", () => {
    expect(phpVersion("8.3.36-1")).toBe("8.3.36");
    expect(phpVersion("7.4.33")).toBe("7.4.33");
    expect(() => phpVersion("8.3.36-rc1")).toThrow();
    expect(() => phpVersion("latest")).toThrow();
  });

  it("builds NTS x64 downloads from releases.json, newest first, with real file names", async () => {
    const php = await buildPhp(fakeSource());
    expect(php.versions.map((v) => v.version)).toEqual(["8.5.12", "8.3.36", "7.4.33"]);

    const v83 = php.versions[1];
    expect(v83.artifacts).toEqual([
      {
        os: "windows",
        arch: "x86_64",
        variant: "nts-vs16",
        format: "zip",
        url: "https://windows.php.net/downloads/releases/php-8.3.36-1-nts-Win32-vs16-x64.zip",
        sha256: "1a097d52b7a68622cdc723073b61cc70caf79c941218ff65b2d8441dbc6b10d4",
        size:
          "https://windows.php.net/downloads/releases/php-8.3.36-1-nts-Win32-vs16-x64.zip".length *
          1000,
        stripComponents: 0,
        requires: ["vcredist-14-x64"],
      },
    ]);
    expect(v83.releasedAt).toBe("2026-10-26");
    expect(php.versions[0].artifacts[0].variant).toBe("nts-vs17");
    expect(php.versions[2].artifacts[0].variant).toBe("nts-vc15");
  });

  it("takes support dates from php.net and only for lines that are offered", async () => {
    const php = await buildPhp(fakeSource());
    expect(Object.keys(php.lines)).toEqual(["8.5", "8.3", "7.4"]);
    expect(php.lines["8.3"]).toEqual({ eol: "2027-12-31", securityOnlySince: "2025-12-31" });
    expect(php.lines["7.4"].eol).toBe("2022-11-28");
  });

  it("finds files that moved to /archives/ and fails loudly when a file is gone", async () => {
    const moved = await buildPhp(
      fakeSource({
        size: (url) => (url.includes("7.4.33") && !url.includes("/archives/") ? null : 4242),
      }),
    );
    expect(moved.versions[2].artifacts[0].url).toBe(
      "https://windows.php.net/downloads/releases/archives/php-7.4.33-nts-Win32-vc15-x64.zip",
    );
    await expect(
      buildPhp(fakeSource({ size: (url) => (url.includes("8.3.36") ? null : 1) })),
    ).rejects.toThrow(/php-8\.3\.36-1-nts-Win32-vs16-x64\.zip is not downloadable/);
  });

  it("writes checksums in lowercase whatever case upstream uses", async () => {
    const releases = json("php-releases.json");
    const lower = releases["8.3"]["nts-vs16-x64"].zip.sha256;
    releases["8.3"]["nts-vs16-x64"].zip.sha256 = lower.toUpperCase();
    const php = await buildPhp({ ...fakeSource(), phpReleases: async () => releases });
    expect(php.versions[1].artifacts[0].sha256).toBe(lower);
  });

  it("fails when a line has no NTS x64 build or a bad checksum, instead of dropping it", async () => {
    const noNts = {
      ...fakeSource(),
      phpReleases: async () => ({ 8.3: { version: "8.3.1", "ts-vs16-x64": {} } }),
    };
    await expect(buildPhp(noNts)).rejects.toThrow(/php 8\.3: no NTS x64 build/);

    const releases = json("php-releases.json");
    releases["8.3"]["nts-vs16-x64"].zip.sha256 = "not a hash";
    await expect(buildPhp({ ...fakeSource(), phpReleases: async () => releases })).rejects.toThrow(
      /not a SHA-256/,
    );
  });
});

describe("node", () => {
  it("keeps the newest stable release of each major and skips prereleases and old majors", () => {
    const newest = newestPerMajor(json("node-index.json"), 16);
    expect(newest.map((n) => n.version)).toEqual(["26.13.0", "24.22.2", "22.22.0", "16.20.2"]);
    expect(newestPerMajor(json("node-index.json"), 0).at(-1).version).toBe("14.21.3");
  });

  it("uses upstream checksums, the official file names and the Node release schedule", async () => {
    const node = await buildNode(fakeSource());
    expect(node.versions.map((v) => v.version)).toEqual([
      "26.13.0",
      "24.22.2",
      "22.22.0",
      "16.20.2",
    ]);

    const v24 = node.versions[1];
    expect(v24.releasedAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(v24.artifacts.map((a) => `${a.os}/${a.arch}/${a.format}`)).toEqual([
      "windows/x86_64/zip",
      "macos/aarch64/tar.gz",
      "macos/x86_64/tar.gz",
      "linux/x86_64/tar.xz",
      "linux/aarch64/tar.xz",
    ]);
    const windows = v24.artifacts[0];
    expect(windows.url).toBe("https://nodejs.org/dist/v24.22.2/node-v24.22.2-win-x64.zip");
    expect(windows.sha256).toBe("ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2");
    expect(windows.stripComponents).toBe(1);
    expect(node.lines["24"].eol).toMatch(/^2028-/);
    expect(node.lines["16"]).toEqual({ eol: "2023-09-11", securityOnlySince: "2022-10-18" });
  });

  it("fails loudly when a listed file cannot be downloaded or none is listed", async () => {
    await expect(
      buildNode(fakeSource({ size: (url) => (url.includes("linux-arm64") ? null : 1) })),
    ).rejects.toThrow(/linux-arm64\.tar\.xz is listed in SHASUMS256\.txt but not downloadable/);
    const empty = { ...fakeSource(), nodeShasums: async () => "" };
    await expect(buildNode(empty)).rejects.toThrow(/lists none of the expected files/);
    await expect(buildNode({ ...fakeSource(), nodeIndex: async () => [] })).rejects.toThrow(
      /no stable release/,
    );
  });
});

describe("generate", () => {
  const clock = (value) => () => value;

  it("builds a valid registry with every runtime, sorted by name", async () => {
    const { registry, errors } = await generate(fakeSource(), {
      now: clock("2026-11-02T00:00:00.000Z"),
    });
    expect(errors).toEqual([]);
    expect(registry.schema).toBe(1);
    expect(registry.generatedAt).toBe("2026-11-02T00:00:00.000Z");
    expect(Object.keys(registry.runtimes)).toEqual(["caddy", "composer", "mariadb", "node", "php"]);
  });

  it("makes the same bytes from the same data and keeps the timestamp until the data changes", async () => {
    const first = (await generate(fakeSource(), { now: clock("2026-11-02T00:00:00.000Z") }))
      .registry;
    const again = (
      await generate(fakeSource(), { previous: first, now: clock("2026-11-03T00:00:00.000Z") })
    ).registry;
    expect(registryText(again)).toBe(registryText(first));

    const changed = (
      await generate(fakeSource({ size: () => 7 }), {
        previous: first,
        now: clock("2026-11-03T00:00:00.000Z"),
      })
    ).registry;
    expect(changed.generatedAt).toBe("2026-11-03T00:00:00.000Z");
  });

  it("keeps a failing runtime's previous entry and reports only that runtime", async () => {
    const { registry: previous } = await generate(fakeSource());
    const broken = {
      ...fakeSource(),
      nodeIndex: async () => {
        throw new Error("nodejs.org is down");
      },
    };

    const { registry, errors } = await generate(broken, { previous });

    expect(errors).toEqual([{ runtime: "node", message: "nodejs.org is down" }]);
    expect(registry.runtimes.node).toEqual(previous.runtimes.node);
    expect(registry.runtimes.php).toEqual(previous.runtimes.php);
  });

  it("with --only refreshes one runtime and leaves the others untouched", async () => {
    const { registry: previous } = await generate(fakeSource());
    const stale = structuredClone(previous);
    stale.runtimes.node.versions.pop();
    stale.runtimes.php.versions.pop();

    const { registry } = await generate(fakeSource(), { previous: stale, only: ["node"] });

    expect(registry.runtimes.node.versions).toHaveLength(previous.runtimes.node.versions.length);
    expect(registry.runtimes.php.versions).toHaveLength(previous.runtimes.php.versions.length - 1);
  });

  it("reports a failing runtime even when there is nothing to keep", async () => {
    const broken = {
      ...fakeSource(),
      phpStates: async () => {
        throw new Error("php.net is down");
      },
    };
    const { registry, errors } = await generate(broken);
    expect(errors.map((e) => e.runtime)).toEqual(["php"]);
    expect(Object.keys(registry.runtimes)).toEqual(["caddy", "composer", "mariadb", "node"]);
  });
});

describe("liveSource", () => {
  const answer =
    (status, headers = {}, body = "") =>
    async () =>
      new Response(body, { status, headers });

  it("asks for the size with HEAD, and treats 404 as missing and anything else wrong as an error", async () => {
    const calls = [];
    const encodings = [];
    const spy = (response) => async (url, init) => {
      calls.push(init.method);
      encodings.push(init.headers["accept-encoding"]);
      return response();
    };
    const ok = liveSource(
      spy(() => new Response(null, { status: 200, headers: { "content-length": "1234" } })),
    );
    expect(await ok.size("https://x.test/a.zip")).toBe(1234);
    expect(calls).toEqual(["HEAD"]);
    expect(encodings).toEqual(["identity"]);

    expect(await liveSource(answer(404)).size("https://x.test/a.zip")).toBeNull();
    await expect(liveSource(answer(503)).size("https://x.test/a.zip")).rejects.toThrow(/HTTP 503/);
    await expect(liveSource(answer(200)).size("https://x.test/a.zip")).rejects.toThrow(
      /did not say how big/,
    );
  });

  it("fails on a bad HTTP status instead of parsing an error page", async () => {
    await expect(liveSource(answer(500, {}, "<html>")).nodeIndex()).rejects.toThrow(/HTTP 500/);
    expect(await liveSource(answer(200, {}, "[1]")).nodeIndex()).toEqual([1]);
  });
});
