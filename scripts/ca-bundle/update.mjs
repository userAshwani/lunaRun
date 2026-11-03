// Refreshes crates/luna-core/data/cacert.pem: Mozilla's list of trusted certificate authorities as published
// by curl (https://curl.se/docs/caextract.html, MPL-2.0). lunaRun gives every PHP it installs this file,
// because PHP on Windows has no certificate store of its own, so Composer and every HTTPS call from PHP
// fail with SSL errors without it (docs/SERVICES.md section 4).
//
//   node scripts/ca-bundle/update.mjs      # or: pnpm ca-bundle
//
// The download is checked against the SHA-256 curl publishes next to it and must look like a certificate
// bundle, so an error page or a truncated file can never be written.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const BUNDLE_URL = "https://curl.se/ca/cacert.pem";
const SUM_URL = "https://curl.se/ca/cacert.pem.sha256";
export const BUNDLE_PATH = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../crates/luna-core/data/cacert.pem",
);

/** `<64 hex>  cacert.pem` to the lowercase hex, or null. */
export function parseSum(text) {
  const match = /^([0-9a-fA-F]{64})\s+\*?cacert\.pem\s*$/m.exec(text);
  return match ? match[1].toLowerCase() : null;
}

/** Problems that make a download not a certificate bundle. Empty when it looks right. */
export function bundleProblems(text) {
  const problems = [];
  const begins = (text.match(/^-----BEGIN CERTIFICATE-----$/gm) ?? []).length;
  const ends = (text.match(/^-----END CERTIFICATE-----$/gm) ?? []).length;
  if (begins !== ends) problems.push(`${begins} certificates begin but ${ends} end`);
  // Mozilla trusts well over a hundred roots. A short file is a truncated or wrong download.
  if (begins < 100) problems.push(`only ${begins} certificates`);
  if (/<html/i.test(text)) problems.push("it is an HTML page");
  return problems;
}

/** Download, verify and return the bundle text. `fetchFn` is replaced in tests. */
export async function fetchBundle(fetchFn = fetch) {
  const get = async (url) => {
    const response = await fetchFn(url, {
      headers: {
        "user-agent": "lunaRun-ca-bundle-updater (+https://github.com/userAshwani/lunaRun)",
      },
    });
    if (!response.ok) throw new Error(`${url} answered HTTP ${response.status}`);
    return Buffer.from(await response.arrayBuffer());
  };
  const bytes = await get(BUNDLE_URL);
  const expected = parseSum((await get(SUM_URL)).toString("utf8"));
  if (!expected) throw new Error(`${SUM_URL} does not contain a SHA-256 for cacert.pem`);
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected) {
    throw new Error(
      `cacert.pem does not match its published SHA-256 (${actual} against ${expected})`,
    );
  }
  const text = bytes.toString("utf8");
  const problems = bundleProblems(text);
  if (problems.length > 0)
    throw new Error(`cacert.pem is not a certificate bundle: ${problems.join(", ")}`);
  return text;
}

async function main() {
  const text = await fetchBundle();
  let previous = "";
  try {
    previous = readFileSync(BUNDLE_PATH, "utf8");
  } catch {
    // first run
  }
  if (previous === text) {
    console.log("cacert.pem is up to date");
    return;
  }
  writeFileSync(BUNDLE_PATH, text);
  const count = (text.match(/BEGIN CERTIFICATE/g) ?? []).length;
  console.log(`wrote ${path.relative(process.cwd(), BUNDLE_PATH)} (${count} certificates)`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
