import { EventEmitter } from "node:events";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { PassThrough } from "node:stream";
import { describe, expect, it } from "vitest";
import { downloadVerified, parseGpgStatus } from "./gpg.mjs";
import { MYSQL_RELEASE_KEY, buildMysql, mysqlCycles } from "./mysql.mjs";
import { fakeSource, json } from "./test-support.mjs";

describe("mysql", () => {
  it("offers long-term-support cycles only, including the end-of-life 8.0", () => {
    // endoflife.date gives `lts` as true, or as the date LTS began (8.0). 9.6 and 26.x are not LTS.
    expect(mysqlCycles(json("endoflife-mysql.json")).map((c) => c.cycle)).toEqual([
      "9.7",
      "8.4",
      "8.0",
    ]);
  });

  it("builds the current release of each cycle with the end-of-life dates endoflife.date gives", async () => {
    const mysql = await buildMysql(fakeSource());
    expect(mysql.versions.map((v) => v.version)).toEqual(["9.7.4", "8.4.13", "8.0.46"]);
    expect(mysql.lines).toEqual({
      9.7: { eol: "2034-04-30" },
      8.4: { eol: "2032-04-30" },
      "8.0": { eol: "2026-04-30" },
    });
    expect(mysql.versions[1].releasedAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("uses MySQL's CDN names, pins the SHA-256 of the verified download and strips the wrapper folder", async () => {
    const mysql = await buildMysql(fakeSource());
    const v84 = mysql.versions[1];
    expect(v84.artifacts.map((a) => `${a.os}/${a.arch}/${a.format}`)).toEqual([
      "windows/x86_64/zip",
      "macos/aarch64/tar.gz",
      "macos/x86_64/tar.gz",
      "linux/x86_64/tar.xz",
    ]);
    const windows = v84.artifacts[0];
    const url = "https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.13-winx64.zip";
    expect(windows).toEqual({
      os: "windows",
      arch: "x86_64",
      format: "zip",
      url,
      sha256: createHash("sha256").update(url).digest("hex"),
      size: url.length * 1000,
      stripComponents: 1,
      requires: ["vcredist-14-x64"],
    });
    expect(v84.artifacts[3].url).toContain("linux-glibc2.28-x86_64.tar.xz");
  });

  it("leaves out a build MySQL did not make and never downloads it", async () => {
    const downloaded = [];
    const base = fakeSource();
    const source = fakeSource({
      size: (url) => (url.includes("8.4.13-macos15-x86_64") ? null : url.length * 1000),
      downloadVerified: async (url, sig) => {
        downloaded.push(url);
        return base.downloadVerified(url, sig);
      },
    });
    const mysql = await buildMysql(source);
    expect(mysql.versions[1].artifacts.map((a) => `${a.os}/${a.arch}`)).toEqual([
      "windows/x86_64",
      "macos/aarch64",
      "linux/x86_64",
    ]);
    expect(downloaded.some((url) => url.includes("8.4.13-macos15-x86_64"))).toBe(false);
  });

  it("refuses a download that is not signed by the pinned MySQL release key", async () => {
    const base = fakeSource();
    const answer = (change) =>
      fakeSource({
        downloadVerified: async (url, sig) => ({
          ...(await base.downloadVerified(url, sig)),
          ...change,
        }),
      });
    for (const signer of [null, "A".repeat(40)]) {
      await expect(buildMysql(answer({ signer }))).rejects.toThrow(
        new RegExp(`not signed by the MySQL release key ${MYSQL_RELEASE_KEY}`),
      );
    }
    await expect(buildMysql(answer({ size: 5 }))).rejects.toThrow(/the server said/);
  });

  it("does not download again what the previous registry already verified", async () => {
    const first = await buildMysql(fakeSource());
    const never = fakeSource({
      downloadVerified: async () => {
        throw new Error("must not download");
      },
      size: () => {
        throw new Error("must not even ask");
      },
    });
    expect(await buildMysql(never, { previous: first })).toEqual(first);

    const downloaded = [];
    const base = fakeSource();
    const spy = fakeSource({
      downloadVerified: async (url, sig) => {
        downloaded.push(url);
        return base.downloadVerified(url, sig);
      },
    });
    const previous = { ...first, versions: first.versions.filter((v) => v.version !== "8.4.13") };
    expect(await buildMysql(spy, { previous })).toEqual(first);
    expect(downloaded).toHaveLength(4);
    expect(downloaded.every((url) => url.includes("mysql-8.4.13-"))).toBe(true);
  });

  it("fails loudly when there is no LTS cycle or no file exists", async () => {
    const none = { ...fakeSource(), mysqlCycles: async () => [{ cycle: "26.10", lts: false }] };
    await expect(buildMysql(none)).rejects.toThrow(/no long-term-support cycle/);
    await expect(buildMysql(fakeSource({ size: () => null }))).rejects.toThrow(
      /none of the expected files exists on cdn\.mysql\.com/,
    );
  });
});

describe("gpg status", () => {
  const good = [
    "[GNUPG:] NEWSIG",
    "[GNUPG:] KEY_CONSIDERED BCA43417C3B485DD128EC6D4B7B3B788A8D3785C 0",
    "[GNUPG:] SIG_ID abc 2026-10-01 1790851563",
    "[GNUPG:] GOODSIG B7B3B788A8D3785C MySQL Release Engineering <mysql-build@oss.oracle.com>",
    "[GNUPG:] VALIDSIG BCA43417C3B485DD128EC6D4B7B3B788A8D3785C 2026-10-01 1790851563 0 4 0 1 8 00 BCA43417C3B485DD128EC6D4B7B3B788A8D3785C",
    "[GNUPG:] TRUST_UNDEFINED 0 pgp",
  ].join("\n");

  it("returns the signer's fingerprint for a good signature by a valid key", () => {
    expect(parseGpgStatus(good)).toBe(MYSQL_RELEASE_KEY);
  });

  it("returns null for an expired key, a bad signature, a missing key or no verdict", () => {
    const withoutGood = good.replace(/.*GOODSIG.*\n/, "");
    expect(parseGpgStatus(withoutGood)).toBeNull(); // VALIDSIG alone is not enough
    expect(parseGpgStatus(good.replace("GOODSIG", "EXPKEYSIG"))).toBeNull();
    expect(parseGpgStatus(`${good}\n[GNUPG:] BADSIG B7B3B788A8D3785C x`)).toBeNull();
    expect(parseGpgStatus(`${good}\n[GNUPG:] ERRSIG B7B3B788A8D3785C 1 8 00 1 9`)).toBeNull();
    expect(parseGpgStatus("[GNUPG:] NO_PUBKEY B7B3B788A8D3785C")).toBeNull();
    expect(parseGpgStatus("")).toBeNull();
    expect(parseGpgStatus(good.replace(/ [0-9A-F]{40}$/m, " not-a-fingerprint"))).toBeNull();
  });

  it("takes the primary key's fingerprint, not a subkey's", () => {
    const subkey = "1".repeat(40);
    const status = good.replace(/VALIDSIG [0-9A-F]{40}/, `VALIDSIG ${subkey}`);
    expect(parseGpgStatus(status)).toBe(MYSQL_RELEASE_KEY);
  });
});

describe("downloadVerified", () => {
  const body = Buffer.from("a pretend mysql archive".repeat(5000));
  const status = `[GNUPG:] GOODSIG B7B3B788A8D3785C x\n[GNUPG:] VALIDSIG ${MYSQL_RELEASE_KEY} 2026-10-01 1 0 4 0 1 8 00 ${MYSQL_RELEASE_KEY}\n`;

  const get = async (url) =>
    new Response(url.endsWith(".asc") ? "-----BEGIN PGP SIGNATURE-----" : body);

  /** A stand-in for the gpg command: records what it was given and answers like gpg's status output. */
  function fakeGpg(calls, answer = status) {
    return (command, args) => {
      const child = new EventEmitter();
      child.stdin = new PassThrough();
      child.stdout = new PassThrough();
      child.stderr = new PassThrough();
      const received = [];
      child.stdin.on("data", (chunk) => received.push(chunk));
      child.stdin.on("end", () => {
        calls.push({ command, args, input: Buffer.concat(received) });
        child.stdout.end(args.includes("--verify") ? answer : "");
        setImmediate(() => child.emit("close", 0));
      });
      return child;
    };
  }

  it("streams the file through gpg and a hash, and reports who signed it", async () => {
    const calls = [];
    const got = await downloadVerified("https://x.test/f.tar.xz", "https://x.test/f.tar.xz.asc", {
      get,
      keyText: "PUBLIC KEY",
      spawnFn: fakeGpg(calls),
    });
    expect(got).toEqual({
      sha256: createHash("sha256").update(body).digest("hex"),
      size: body.length,
      signer: MYSQL_RELEASE_KEY,
    });

    const [imported, verified] = calls;
    expect(imported.args).toContain("--import");
    expect(imported.input.toString()).toBe("PUBLIC KEY");
    expect(verified.args).toEqual(expect.arrayContaining(["--verify", "--status-fd", "1", "-"]));
    expect(verified.input.equals(body)).toBe(true); // gpg saw exactly the downloaded bytes
  });

  it("uses a throw-away key ring and removes it afterwards", async () => {
    const calls = [];
    await downloadVerified("https://x.test/f", "https://x.test/f.asc", {
      get,
      keyText: "k",
      spawnFn: fakeGpg(calls),
    });
    const homes = calls.map((c) => c.args[c.args.indexOf("--homedir") + 1]);
    expect(homes[0]).toBe(homes[1]);
    expect(homes[0]).toMatch(/lunarun-gpg-/);
    expect(existsSync(homes[0])).toBe(false);
  });

  it("returns no signer when gpg does not vouch for the file", async () => {
    const got = await downloadVerified("https://x.test/f", "https://x.test/f.asc", {
      get,
      keyText: "k",
      spawnFn: fakeGpg([], "[GNUPG:] BADSIG B7B3B788A8D3785C x\n"),
    });
    expect(got.signer).toBeNull();
  });

  it("says that gpg is needed when it is not installed", async () => {
    const missing = () => {
      const child = new EventEmitter();
      child.stdin = new PassThrough();
      child.stdout = new PassThrough();
      child.stderr = new PassThrough();
      process.nextTick(() =>
        child.emit("error", Object.assign(new Error("spawn gpg"), { code: "ENOENT" })),
      );
      return child;
    };
    await expect(
      downloadVerified("https://x.test/f", "https://x.test/f.asc", {
        get,
        keyText: "k",
        spawnFn: missing,
      }),
    ).rejects.toThrow(/gpg is needed to verify this download/);
  });
});
