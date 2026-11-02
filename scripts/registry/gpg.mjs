// Verifying a download against a detached OpenPGP signature with the `gpg` command, for upstreams that
// publish signatures but no usable checksum (MySQL). The file is streamed through gpg and a SHA-256 at the
// same time, so a 900 MB archive never has to be stored.
//
// Trust rests on one thing only: the **fingerprint the caller pins**. The key itself may come from anywhere
// (a keyserver), because a signature only counts when gpg reports exactly that fingerprint as the signer.
// This is generator-side tooling. The app itself never runs gpg.

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

/**
 * The fingerprint of the signer when gpg's `--status-fd` output shows a good signature by a key that is
 * valid (not expired, not revoked), else null. `EXPKEYSIG`, `REVKEYSIG`, `BADSIG`, `ERRSIG` and a missing
 * `GOODSIG` are all null, so the caller can only ever see a clean, current signature.
 */
export function parseGpgStatus(status) {
  const lines = status.split(/\r?\n/).filter((line) => line.startsWith("[GNUPG:] "));
  const kinds = new Set(lines.map((line) => line.split(" ")[1]));
  if (!kinds.has("GOODSIG") || kinds.has("BADSIG") || kinds.has("ERRSIG")) return null;
  const valid = lines.filter((line) => line.split(" ")[1] === "VALIDSIG");
  if (valid.length !== 1) return null;
  // VALIDSIG <fingerprint> <date> <timestamp> <expire> <version> <reserved> <pubkey-algo> <hash-algo> <class> <primary-key-fingerprint>
  const fields = valid[0].split(" ");
  const primary = fields[fields.length - 1];
  return /^[0-9A-F]{40}$/.test(primary) ? primary : null;
}

/** Run gpg to completion, optionally feeding it `input`. Never rejects on a non-zero exit, the caller reads the output. */
function run(spawnFn, command, args, input) {
  return new Promise((resolve, reject) => {
    const child = spawnFn(command, args, { stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.resume();
    child.stdin.on("error", () => {}); // gpg may exit before reading everything, its status says why
    child.on("error", (error) =>
      reject(
        error.code === "ENOENT"
          ? new Error(
              `${command} was not found: gpg is needed to verify this download (set GPG to its path)`,
            )
          : error,
      ),
    );
    child.on("close", (code) => resolve({ code, stdout }));
    if (input !== undefined) child.stdin.end(input);
  });
}

/**
 * Download `url` once, verify it against the detached signature at `sigUrl` and return its SHA-256 and size.
 *
 * @param {object} options
 * @param {(url: string) => Promise<Response>} options.get fetches with the generator's headers and fails on bad status
 * @param {string} options.keyText the ASCII-armoured public key to verify with
 * @param {string} [options.gpg] the gpg command, `gpg` by default
 * @param {typeof spawn} [options.spawnFn] replaced in tests
 * @returns {Promise<{ sha256: string, size: number, signer: string | null }>} `signer` is the verified fingerprint or null
 */
export async function downloadVerified(
  url,
  sigUrl,
  { get, keyText, gpg = "gpg", spawnFn = spawn },
) {
  const home = await mkdtemp(path.join(os.tmpdir(), "lunarun-gpg-"));
  try {
    const base = ["--batch", "--homedir", home];
    await run(spawnFn, gpg, [...base, "--import"], keyText);
    const sigPath = path.join(home, "download.asc");
    await writeFile(sigPath, await (await get(sigUrl)).text());

    const response = await get(url);
    const child = spawnFn(gpg, [...base, "--status-fd", "1", "--verify", sigPath, "-"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    let status = "";
    child.stdout.on("data", (chunk) => (status += chunk));
    child.stderr.resume();
    child.stdin.on("error", () => {});
    const closed = new Promise((resolve, reject) => {
      child.on("error", reject);
      child.on("close", resolve);
    });

    const sha256 = createHash("sha256");
    let size = 0;
    for await (const chunk of response.body) {
      sha256.update(chunk);
      size += chunk.length;
      if (!child.stdin.write(chunk)) await once(child.stdin, "drain").catch(() => {});
    }
    child.stdin.end();
    await closed.catch((error) => {
      throw error.code === "ENOENT"
        ? new Error(
            `${gpg} was not found: gpg is needed to verify this download (set GPG to its path)`,
          )
        : error;
    });
    return { sha256: sha256.digest("hex"), size, signer: parseGpgStatus(status) };
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}
