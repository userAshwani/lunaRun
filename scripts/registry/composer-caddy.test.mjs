import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { buildCaddy, newestPerMinor } from "./caddy.mjs";
import { buildComposer, composerLines, composerReleases } from "./composer.mjs";
import { liveSource } from "./generate.mjs";
import { parseChecksums } from "./lib.mjs";
import { fakeSource, json } from "./test-support.mjs";

describe("composer", () => {
  it("lists real releases only, once each, newest first", () => {
    // `preview` and `snapshot` also appear in the file, and 2.2.31 is named under four keys.
    expect(composerReleases(json("composer-versions.json"))).toEqual([
      { version: "2.11.1", path: "/download/2.11.1/composer.phar" },
      { version: "2.2.31", path: "/download/2.2.31/composer.phar" },
      { version: "1.10.28", path: "/download/1.10.28/composer.phar" },
    ]);
  });

  it("ignores the preview and snapshot channels and entries whose path and version disagree", () => {
    const entry = (version, path = `/download/${version}/composer.phar`) => ({ version, path });
    const data = {
      stable: [entry("2.0.0"), entry("2.0.1", "/download/2.0.5/composer.phar")],
      preview: [entry("2.99.0")],
      snapshot: [entry("2.98.0")],
      2: [entry("2.0.0")],
    };
    expect(composerReleases(data).map((r) => r.version)).toEqual(["2.0.0"]);
  });

  it("takes end-of-life dates only where Composer publishes one", () => {
    expect(composerLines(json("composer-versions.json"))).toEqual({
      1: { eol: "2026-05-30" },
      2.2: { eol: "2026-12-31" },
    });
  });

  it("lists the one file for every platform, with the checksum Composer publishes", async () => {
    const composer = await buildComposer(fakeSource());
    expect(composer.versions.map((v) => v.version)).toEqual(["2.11.1", "2.2.31", "1.10.28"]);

    const latest = composer.versions[0];
    expect(latest.artifacts.map((a) => `${a.os}/${a.arch}`)).toEqual([
      "windows/x86_64",
      "macos/aarch64",
      "macos/x86_64",
      "linux/x86_64",
      "linux/aarch64",
    ]);
    for (const artifact of latest.artifacts) {
      expect(artifact).toMatchObject({
        format: "file",
        url: "https://getcomposer.org/download/2.11.1/composer.phar",
        sha256: "6f1b891e1d4927d62740276c540fb490e47689e19e102c49e8da849794882f8c",
        stripComponents: 0,
      });
    }
  });

  it("fails loudly on a missing checksum or a file that is gone", async () => {
    const noChecksum = { ...fakeSource(), composerChecksum: async () => "no checksum here\n" };
    await expect(buildComposer(noChecksum)).rejects.toThrow(/not a SHA-256/);
    await expect(buildComposer(fakeSource({ size: () => null }))).rejects.toThrow(
      /is not downloadable/,
    );
    const empty = { ...fakeSource(), composerVersions: async () => ({ stable: [] }) };
    await expect(buildComposer(empty)).rejects.toThrow(/lists no release/);
  });
});

describe("caddy", () => {
  const tags = (releases) => releases.map((r) => r.version);

  it("keeps the newest stable release of the latest minor lines only", () => {
    const releases = json("caddy-releases.json");
    // v2.11.0-beta.2 is a prerelease, v2.11.6 is older than v2.11.7, v2.8.4 is beyond three lines.
    expect(tags(newestPerMinor(releases, 3))).toEqual(["2.11.7", "2.10.2", "2.9.1"]);
    expect(tags(newestPerMinor(releases, 1))).toEqual(["2.11.7"]);
    expect(tags(newestPerMinor(releases, 9))).toHaveLength(4);
    const draft = [{ tag_name: "v3.0.0", draft: true, prerelease: false, assets: [] }];
    expect(newestPerMinor(draft, 3)).toEqual([]);
  });

  it("pins the SHA-256 of a download whose SHA-512 matches the authors' checksums", async () => {
    const caddy = await buildCaddy(fakeSource());
    expect(caddy.versions.map((v) => v.version)).toEqual(["2.11.7", "2.10.2", "2.9.1"]);
    expect(caddy.lines).toEqual({});

    const latest = caddy.versions[0];
    expect(latest.releasedAt).toBe("2026-10-03");
    expect(latest.artifacts.map((a) => `${a.os}/${a.arch}/${a.format}`)).toEqual([
      "windows/x86_64/zip",
      "macos/aarch64/tar.gz",
      "macos/x86_64/tar.gz",
      "linux/x86_64/tar.gz",
      "linux/aarch64/tar.gz",
    ]);
    expect(latest.artifacts[0]).toEqual({
      os: "windows",
      arch: "x86_64",
      format: "zip",
      url: "https://github.com/caddyserver/caddy/releases/download/v2.11.7/caddy_2.11.7_windows_amd64.zip",
      sha256: "0a1edc0b799512051c57071ce0e798d3f2cf67dc98171366f1d2b326072e3b06",
      size: 18479498,
      stripComponents: 0,
    });
  });

  it("refuses a download that is not what the authors published", async () => {
    const real = fakeSource();
    const tamper = (change) =>
      fakeSource({ hash: async (url) => ({ ...(await real.hash(url)), ...change }) });

    await expect(buildCaddy(tamper({ sha512: "0".repeat(128) }))).rejects.toThrow(
      /does not match the SHA-512 the Caddy authors published/,
    );
    await expect(buildCaddy(tamper({ sha256: "1".repeat(64) }))).rejects.toThrow(
      /does not match the SHA-256 digest GitHub shows/,
    );
    await expect(buildCaddy(tamper({ size: 5 }))).rejects.toThrow(/GitHub says 18479498/);
  });

  it("does not download again what the previous registry already verified", async () => {
    const first = await buildCaddy(fakeSource());

    const nothing = fakeSource({
      hash: async () => {
        throw new Error("must not download");
      },
    });
    expect(await buildCaddy(nothing, { previous: first })).toEqual(first);

    // A release that is new since the last run is downloaded and checked, the rest is reused.
    const downloads = [];
    const real = fakeSource();
    const spy = fakeSource({
      hash: async (url) => {
        downloads.push(url);
        return real.hash(url);
      },
    });
    const previous = { ...first, versions: first.versions.filter((v) => v.version !== "2.9.1") };
    const rebuilt = await buildCaddy(spy, { previous });
    expect(rebuilt).toEqual(first);
    expect(downloads).toHaveLength(5);
    expect(downloads.every((url) => url.includes("v2.9.1"))).toBe(true);
  });

  it("fails loudly when a release and its checksums disagree or it has no usable file", async () => {
    const stripped = json("caddy-releases.json");
    stripped[0].assets = stripped[0].assets.filter((a) => !a.name.includes("windows_amd64"));
    await expect(
      buildCaddy({ ...fakeSource(), caddyReleases: async () => stripped }),
    ).rejects.toThrow(
      /caddy_2\.11\.7_windows_amd64\.zip is missing from the release or from its checksums/,
    );

    // Checksums without assets, and assets without checksums, are both a broken release.
    const noChecksums = { ...fakeSource(), caddyChecksums: async () => "" };
    await expect(buildCaddy(noChecksums)).rejects.toThrow(
      /missing from the release or from its checksums/,
    );

    const empty = json("caddy-releases.json").map((release) => ({ ...release, assets: [] }));
    const bare = {
      ...fakeSource(),
      caddyReleases: async () => empty,
      caddyChecksums: async () => "",
    };
    await expect(buildCaddy(bare)).rejects.toThrow(/none of the expected files/);
    await expect(buildCaddy({ ...fakeSource(), caddyReleases: async () => [] })).rejects.toThrow(
      /no stable release/,
    );
  });
});

describe("checksum lists", () => {
  it("never reads a SHA-512 list as SHA-256 or the other way round", () => {
    const sha512 = "a".repeat(128);
    const sha256 = "b".repeat(64);
    const list = `${sha512}  big.zip\n${sha256}  small.zip\n`;
    expect([...parseChecksums(list, 128)]).toEqual([["big.zip", sha512]]);
    expect([...parseChecksums(list, 64)]).toEqual([["small.zip", sha256]]);
  });
});

describe("liveSource downloads and GitHub", () => {
  it("hashes a download as it streams, without keeping it", async () => {
    const body = Buffer.from("a pretend caddy archive".repeat(1000));
    const fetchFn = async () => new Response(body, { status: 200 });
    const got = await liveSource(fetchFn).hash("https://x.test/caddy.zip");
    expect(got).toEqual({
      sha256: createHash("sha256").update(body).digest("hex"),
      sha512: createHash("sha512").update(body).digest("hex"),
      size: body.length,
    });
  });

  it("explains a used-up GitHub rate limit instead of a bare 403", async () => {
    const limited = async () =>
      new Response("{}", { status: 403, headers: { "x-ratelimit-remaining": "0" } });
    await expect(liveSource(limited).caddyReleases()).rejects.toThrow(/rate limit.*GITHUB_TOKEN/);
    const forbidden = async () => new Response("{}", { status: 403 });
    await expect(liveSource(forbidden).caddyReleases()).rejects.toThrow(/HTTP 403/);
  });

  it("sends the GitHub token only to api.github.com", async () => {
    const seen = [];
    const fetchFn = async (url, init) => {
      seen.push([new URL(url).hostname, init.headers.authorization]);
      return new Response("[]", { status: 200 });
    };
    const previous = process.env.GITHUB_TOKEN;
    process.env.GITHUB_TOKEN = "test-token";
    try {
      const source = liveSource(fetchFn);
      await source.caddyReleases();
      await source.nodeIndex();
    } finally {
      if (previous === undefined) delete process.env.GITHUB_TOKEN;
      else process.env.GITHUB_TOKEN = previous;
    }
    expect(seen).toEqual([
      ["api.github.com", "Bearer test-token"],
      ["nodejs.org", undefined],
    ]);
  });
});
