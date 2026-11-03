import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { bundleProblems, fetchBundle, parseSum } from "./update.mjs";

const cert = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
const bundle = (count) => `## header\n${cert.repeat(count)}`;
const sha = (text) => createHash("sha256").update(text).digest("hex");

/** A fetch that answers from a map and counts nothing else. */
const server = (answers) => async (url) => {
  const body = answers[url];
  return body === undefined
    ? new Response("", { status: 404 })
    : new Response(body, { status: 200 });
};

describe("parseSum", () => {
  it("reads the sha256sum line", () => {
    expect(parseSum(`${"A".repeat(64)}  cacert.pem\n`)).toBe("a".repeat(64));
    expect(parseSum(`${"b".repeat(64)} *cacert.pem`)).toBe("b".repeat(64));
  });

  it("refuses anything else", () => {
    for (const bad of [
      "",
      "not a sum",
      `${"a".repeat(63)}  cacert.pem`,
      `${"a".repeat(64)}  other.pem`,
    ]) {
      expect(parseSum(bad), bad).toBeNull();
    }
  });
});

describe("bundleProblems", () => {
  it("accepts a bundle of many certificates", () => {
    expect(bundleProblems(bundle(120))).toEqual([]);
  });

  it("rejects a truncated bundle, an unbalanced one and an HTML page", () => {
    expect(bundleProblems(bundle(5)).join()).toContain("only 5 certificates");
    expect(bundleProblems(`${bundle(120)}-----BEGIN CERTIFICATE-----\nMIIB\n`).join()).toContain(
      "begin but",
    );
    expect(bundleProblems("<html><body>rate limited</body></html>").join()).toContain("HTML");
  });
});

describe("fetchBundle", () => {
  const url = "https://curl.se/ca/cacert.pem";
  const sumUrl = "https://curl.se/ca/cacert.pem.sha256";

  it("returns a bundle that matches its published checksum", async () => {
    const text = bundle(120);
    const fetchFn = server({ [url]: text, [sumUrl]: `${sha(text)}  cacert.pem\n` });
    expect(await fetchBundle(fetchFn)).toBe(text);
  });

  it("refuses a bundle that does not match its checksum", async () => {
    const fetchFn = server({
      [url]: bundle(120),
      [sumUrl]: `${sha("something else")}  cacert.pem\n`,
    });
    await expect(fetchBundle(fetchFn)).rejects.toThrow(/does not match its published SHA-256/);
  });

  it("refuses a matching file that is not a certificate bundle", async () => {
    const text = "<html>an error page</html>";
    const fetchFn = server({ [url]: text, [sumUrl]: `${sha(text)}  cacert.pem\n` });
    await expect(fetchBundle(fetchFn)).rejects.toThrow(/not a certificate bundle/);
  });

  it("fails on a bad HTTP status or a missing checksum", async () => {
    await expect(fetchBundle(server({}))).rejects.toThrow(/HTTP 404/);
    await expect(fetchBundle(server({ [url]: bundle(120), [sumUrl]: "garbage" }))).rejects.toThrow(
      /does not contain a SHA-256/,
    );
  });
});
