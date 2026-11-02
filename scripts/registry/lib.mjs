// Shared helpers of the registry generator. Pure functions only, so they are tested without a network.
//
// Plain .mjs on purpose, like the other scripts: it runs on a bare Node with no install step.

const SHA256 = /^[0-9a-f]{64}$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

/** `a.b.c` strings compared number by number: 8.10.0 is newer than 8.9.0. Negative, zero or positive like sort(). */
export function compareVersions(a, b) {
  const left = a.split(".").map(Number);
  const right = b.split(".").map(Number);
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const diff = (left[i] ?? 0) - (right[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

/**
 * A checksum list: one `<hex>  <file name>` per line (`sha256sum` and `sha512sum` format). Returns a Map of
 * file name to lowercase hex. Lines that are not a checksum of exactly `hexLength` digits are ignored, so a
 * SHA-512 list can never be read as SHA-256 by accident.
 */
export function parseChecksums(text, hexLength) {
  const line = new RegExp(`^([0-9a-fA-F]{${hexLength}}) [ *](.+)$`);
  const sums = new Map();
  for (const entry of text.split(/\r?\n/)) {
    const match = line.exec(entry.trim());
    if (match) sums.set(match[2], match[1].toLowerCase());
  }
  return sums;
}

/** SHASUMS256.txt as published by Node.js. */
export const parseShasums = (text) => parseChecksums(text, 64);

/** `2027-12-31T00:00:00+00:00` or `2027-12-31` to `2027-12-31`; throws on anything else. */
export function isoDate(value, what) {
  const day = typeof value === "string" ? value.slice(0, 10) : "";
  if (!DATE.test(day)) throw new Error(`${what}: ${JSON.stringify(value)} is not a date`);
  return day;
}

export function checkedSha256(value, what) {
  const hex = typeof value === "string" ? value.toLowerCase() : "";
  if (!SHA256.test(hex)) throw new Error(`${what}: ${JSON.stringify(value)} is not a SHA-256`);
  return hex;
}

/**
 * The registry rules the app enforces (crates/luna-core/src/registry), repeated here so a bad file is
 * refused before it is written. The Rust parser stays the final judge: a test parses the real file.
 * @returns {string[]} problems, empty when the registry is fine
 */
export function validateRegistry(registry) {
  const problems = [];
  if (registry.schema !== 1) problems.push(`schema must be 1, got ${registry.schema}`);
  for (const [id, runtime] of Object.entries(registry.runtimes)) {
    if (!/^[a-z][a-z0-9-]*$/.test(id))
      problems.push(`runtime id ${JSON.stringify(id)} is not a safe name`);
    const seen = new Set();
    for (const release of runtime.versions) {
      const at = `${id} ${release.version}`;
      if (!/^\d+(\.\d+){1,3}(-[0-9A-Za-z.-]+)?$/.test(release.version))
        problems.push(`${at}: bad version`);
      if (seen.has(release.version)) problems.push(`${at}: listed twice`);
      seen.add(release.version);
      if (release.artifacts.length === 0) problems.push(`${at}: no downloads`);
      const targets = new Set();
      for (const artifact of release.artifacts) {
        const target = `${artifact.os}/${artifact.arch}/${artifact.variant ?? ""}`;
        if (targets.has(target)) problems.push(`${at}: two downloads for ${target}`);
        targets.add(target);
        if (!artifact.url.startsWith("https://"))
          problems.push(`${at} ${target}: url is not https`);
        if (!SHA256.test(artifact.sha256)) problems.push(`${at} ${target}: bad sha256`);
        if (!Number.isInteger(artifact.size) || artifact.size <= 0)
          problems.push(`${at} ${target}: bad size`);
      }
    }
  }
  return problems;
}

/** Stable text for a registry: keys are inserted in a fixed order by the builders, so equal data is equal text. */
export function registryText(registry) {
  return `${JSON.stringify(registry, null, 2)}\n`;
}
