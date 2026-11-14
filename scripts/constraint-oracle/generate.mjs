// Makes the golden fixtures of the constraint engine by asking the REAL implementations:
//
//   crates/luna-core/tests/fixtures/constraints/composer.json   <- Composer's own semver library (composer.phar)
//   crates/luna-core/tests/fixtures/constraints/npm.json        <- node-semver (the `semver` npm package)
//
// Each file lists, for a long table of constraints, whether the real implementation accepts it and which of a list of
// versions satisfy it. The Rust tests (crates/luna-core/tests/constraints_oracle.rs) hold the lunaRun parsers to exactly
// those answers. Run it again when a table below changes or when Composer or node-semver change their behaviour.
//
//   PHP=C:/php/php.exe COMPOSER_PHAR=<path to composer.phar> SEMVER_PATH=<folder of the semver package> node scripts/constraint-oracle/generate.mjs
//
// COMPOSER_PHAR must be the official file: it is checked against the SHA-256 in registry/registry.json before it runs.
// SEMVER_PATH defaults to the `semver` package found by Node (pnpm keeps one under node_modules/.pnpm/semver@<version>/node_modules/semver).
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";

const repo = path.resolve(import.meta.dirname, "..", "..");
const outDir = path.join(repo, "crates", "luna-core", "tests", "fixtures", "constraints");
fs.mkdirSync(outDir, { recursive: true });

// ------------------------------------------------------------------------------------------------ the tables
const product = (a, b) => a.flatMap((x) => b.map((y) => x + y));

const composerVersions = [
  "0.0.0",
  "0.0.1",
  "0.0.3",
  "0.0.4",
  "0.1.0",
  "0.2.9",
  "0.3.0",
  "0.3.5",
  "0.4.0",
  "1.0.0",
  "1.2.3",
  "1.2.3.4",
  "1.2.3.5",
  "1.2.4",
  "1.3.0",
  "2.0.0",
  "7.0.0",
  "7.4.0",
  "7.4.33",
  "7.4.34",
  "8.0.0",
  "8.0.30",
  "8.1.0",
  "8.1.2",
  "8.1.34",
  "8.2.0",
  "8.2.1",
  "8.2.28",
  "8.3.0",
  "8.3.14",
  "8.4.0",
  "8.4.12",
  "8.5.0",
  "9.0.0",
  "10.0.0",
  "10.5.2",
  "11.0.0",
  "24.22.2",
  // the versions that tell real constraints apart
  "5.5.9",
  "5.6.0",
  "5.6.40",
  "7.1.3",
  "7.2.4",
  "7.2.5",
  "7.3.0",
  "7.3.33",
  "7.4.19",
  "7.4.20",
  "8.0.2",
  "8.0.6",
  "8.0.7",
  "8.1.99",
  "8.3.99",
];

const composerPartials = [
  "8",
  "8.0",
  "8.1",
  "8.1.0",
  "8.1.2",
  "0",
  "0.0",
  "0.3",
  "0.0.3",
  "0.0.0",
  "1.2.3.4",
  "7.4",
  "7.4.33",
  "10",
  "10.0",
  "1.2",
  "1.2.3",
];
const composerOps = ["^", "~", ">=", ">", "<=", "<", "=", "==", "!=", "<>", ""];

const composerConstraints = [
  ...product(composerOps, composerPartials),
  // a leading v, and spaces after the operator
  "v8.1.0",
  "^v8.1",
  "=v8.1.0",
  ">=v8.1",
  ">= 8.1",
  "> 8.1",
  "<= 8.3",
  "< 8.4",
  "!= 8.2.0",
  "^ 8.1",
  "~ 8.1",
  "= 8.1.0",
  // wildcards
  "*",
  "x",
  "X",
  "8.*",
  "8.x",
  "8.X",
  "8.1.*",
  "8.1.x",
  "8.1.2.*",
  "0.*",
  "0.0.*",
  "1.*.*",
  "8.*.*",
  ">=8.*",
  "^8.*",
  "~8.*",
  "!=8.*",
  "<8.*",
  "*.*",
  // hyphen ranges
  "8.1 - 8.3",
  "8.1.0 - 8.3.99",
  "8 - 9",
  "8.1 - 8.3.5",
  "7.4 - 8",
  "1.0.0 - 2.0.0",
  "8.3 - 8.1",
  "8.1 -8.3",
  "8.1- 8.3",
  "8.1-8.3",
  "8.1 - 8.3 || 7.4",
  // AND
  ">=8.1 <8.4",
  ">=8.1, <8.4",
  ">=8.1,<8.4",
  ">=8.1  <8.4",
  ">= 8.1 < 8.4",
  "^8.1 !=8.2.0",
  ">8.1 <=8.3.5",
  ">=7.4 <8.0 || >=8.1",
  "<8.0 >=7.0",
  "^7.4 ^8.0",
  ">=8.1 <8.1",
  "8.1.0 8.1.0",
  ">=1 <=1",
  ">=8.1\t<8.4",
  // OR
  "^7.4 || ^8.0",
  "^7.4 | ^8.0",
  "^7.4|^8.0",
  "^7.3|^8.0",
  "^7.4||^8.0",
  "~5.6|~7.0",
  "8.1.0 || 8.2.0",
  "^8.1 || 7.4.*",
  ">=7.0 || <5",
  "^7 || ^8 || ^9",
  // stability flags and branches
  "^8.1@dev",
  "^8.1@stable",
  ">=8.1@beta",
  "8.1.0@dev",
  "dev-main",
  "dev-master",
  "^8.1 || dev-main",
  "dev-main as 8.1.0",
  "8.1.x-dev",
  "dev-feature/x",
  // constraints as written in real composer.json files
  "^7.3|^8.0",
  "^8.0.2",
  "^8.1",
  "^8.2",
  ">=8.2",
  "^7.2.5|^8.0",
  ">=7.1.3",
  "^7.1.3|^8.0",
  ">=5.6.4",
  "~5.6|~7.0",
  ">=7.0",
  "^8.1|^8.2",
  "7.4.*",
  "^7.4 || ^8.0",
  ">=5.5.9",
  "^5.5.9 || ^7.0",
  "^8.0 <8.5",
  ">=7.2.5 <8.4",
  "8.2.*",
  "~8.2.0",
  "^8.3.0",
  "8.1.* || 8.2.* || 8.3.*",
  ">=8.1 <8.3 || >=8.4",
  "^7.4.20 || ^8.0.7",
  "^8.2.0",
  "~8.1.2",
  "^0.3",
  "^0.0.3",
  // more spellings of separators, flags and numbers
  ">=8.1 ,<8.4",
  ">=8.1 , <8.4",
  "  >=8.1 <8.4  ",
  "^7.4||^8.0",
  "^7.4 ||^8.0",
  "^7.4|| ^8.0",
  "^8.1 !=8.2.0",
  "8.1.2.*",
  "~0.3",
  "~10.0",
  "<=0",
  "~8",
  "~1.2.3.4",
  "^8.1 ||",
  "|| ^8.1",
  "^8.1 || || ^9",
  "^8.1 ,",
  "^8.1@nope",
  "@dev",
  "8.*-dev",
  "8.*.1",
  "99999999999999999999999.1",
  "^8.1|||^9",
  "8.1 - 8.3 - 9",
  "1.x-dev",
  "dev-main dev-other",
  "8.1.0-beta",
  ">=8.2.0-RC1",
  "^8.1.0-beta2",
  "<8.4 || >=8.4.0-rc1",
  "8.1.0-foo",
  "8.1.0-8",
  "8.1.0-dev",
  "@stable",
  "*@dev",
  "^8.1@dev || ^7",
  "8.x-dev",
  "8.*-dev",
  "1.2.*-dev",
  "dev-main@dev",
  "8.1.2.x-dev",
  ">=dev-main",
  "8.1.x-DEV",
  // what must be refused
  "",
  "  ",
  "abc",
  "^",
  "~",
  ">=",
  "8..1",
  "8.1.",
  "^^8.1",
  "8.1 -",
  "- 8.1",
  ">=>8.1",
  "8.1.2.3.4",
  "^8.1 abc",
  "%",
  "8,1",
  "^8.1,,<9",
  ",^8.1",
  "||",
  "|",
  "8.1 as",
  "^a.b",
];

const npmVersions = [
  "0.0.0",
  "0.0.1",
  "0.0.3",
  "0.0.4",
  "0.1.0",
  "0.2.2",
  "0.2.3",
  "0.2.4",
  "0.3.0",
  "0.10.0",
  "1.0.0",
  "1.2.2",
  "1.2.3",
  "1.2.4",
  "1.2.5",
  "1.3.0",
  "1.9.9",
  "2.0.0",
  "2.3.4",
  "2.4.0",
  "3.0.0",
  "4.0.0",
  "5.0.0",
  "5.1.0",
  "6.0.0",
  "7.2.3",
  "7.2.4",
  "8.0.0",
  "10.0.0",
  "12.22.0",
  "14.13.1",
  "16.0.0",
  "16.13.0",
  "16.14.0",
  "18.0.0",
  "18.17.0",
  "18.17.1",
  "18.18.0",
  "19.0.0",
  "20.0.0",
  "20.5.0",
  "20.11.0",
  "20.11.1",
  "22.1.0",
  "24.22.2",
  // real Node releases that tell real engines fields apart
  "0.10.0",
  "0.12.18",
  "4.9.1",
  "6.9.0",
  "6.17.1",
  "8.17.0",
  "10.24.1",
  "12.0.0",
  "12.17.0",
  "14.0.0",
  "14.17.0",
  "14.18.0",
  "16.9.0",
  "16.10.0",
  "16.20.2",
  "18.12.0",
  "18.13.0",
  "18.20.4",
  "20.9.0",
  "20.19.0",
  "22.0.0",
  "22.12.0",
  "23.0.0",
  // pre-releases (dotted identifiers, which Node and lunaRun order the same way)
  "1.2.3-alpha",
  "1.2.3-alpha.1",
  "1.2.3-beta",
  "1.2.3-beta.2",
  "1.2.3-rc.1",
  "1.2.4-alpha",
  "1.2.4-alpha.2",
  "2.0.0-rc.1",
  "2.0.0-0",
  "0.2.3-beta",
  "18.0.0-pre.1",
  "1.0.0-0",
];

const npmPartials = [
  "1",
  "1.2",
  "1.2.3",
  "0",
  "0.0",
  "0.2.3",
  "0.0.3",
  "0.0.0",
  "18",
  "18.17",
  "18.17.1",
  "x",
  "1.x",
  "1.2.x",
  "*",
  "1.2.*",
  "X",
  "1.X",
  "0.x",
  "0.0.x",
  "1.x.x",
  "10",
  "8",
];
const npmOps = ["^", "~", ">=", ">", "<=", "<", "=", "", "v", "=v", "~>"];

const npmRanges = [
  ...product(npmOps, npmPartials),
  ">= 1.2.3",
  "^ 1.2.3",
  "~ 1.2",
  "< 2",
  "<= 2.0.0",
  "> 1.2.3",
  "= 1.2.3",
  "~> 1.2.3",
  ">=  1.2.3",
  // hyphen ranges
  "1.2.3 - 2.3.4",
  "1.2 - 2.3.4",
  "1.2.3 - 2.3",
  "1.2.3 - 2",
  "1 - 2",
  "1.x - 2.x",
  "* - 2",
  "1 - *",
  "2.3.4 - 1.2.3",
  "1.2.3-2.3.4",
  "1.2.3 -2.3.4",
  "1.2.3- 2.3.4",
  "0 - 0",
  "1.2.3 - 2.3.4 || 5",
  // AND, OR
  ">=1.2.3 <2.0.0",
  ">=1.2.3 <1.3.0 || >=2",
  ">=1.2.3  <2.0.0",
  ">=1.2.3,<2.0.0",
  ">1.2.3 <1.2.3",
  ">=1.0.0 <=1.0.0",
  "^1.2.3 ^1.2.4",
  "~1.2.3 >=1.2.5",
  "<1.0.0 >2.0.0",
  ">=18 <19",
  ">=1.2.3\t<2.0.0",
  "^16.14.0 || >=18.0.0",
  "1.x || >=2.5.0 || 5.0.0 - 7.2.3",
  "<1.2.3 || >4",
  "18.x || 20.x",
  "^12.20.0 || ^14.13.1 || >=16.0.0",
  "||",
  "1 ||",
  "|| 1",
  "^18.17.0 || >=20.5.0",
  ">=16.13.0 <17 || >=18",
  "1.2.3||2.0.0",
  "1.2.3 ||2.0.0",
  // as written in real package.json "engines" fields
  ">=18",
  ">=14.0.0",
  ">=12.22.0",
  ">= 8",
  ">=10 <15",
  "20.x",
  "*",
  "",
  ">=16",
  "^20.0.0",
  "18 || 20 || 22",
  "^18 || ^20",
  ">=18.18",
  "~20.11.0",
  ">=0.10",
  ">=6.9.0 <7 || >=8",
  "latest",
  "  >=18  ",
  // pre-releases
  "^1.2.3-alpha.1",
  "~1.2.3-beta",
  "1.2.3-beta",
  ">=1.2.3-rc.1",
  "<2.0.0",
  "<2.0.0-0",
  ">1.2.3-alpha",
  ">=1.2.3-alpha <1.2.4",
  "1.2.3-alpha.1 - 1.2.4",
  "^0.2.3-beta",
  ">=1.2.3-0",
  "^1.0.0-0",
  "1.2.3-alpha || 1.2.4-alpha",
  "^1.2.3-beta.2 || ^2.0.0-rc.1",
  // more spellings
  "1.x.3",
  "1.2.x-beta",
  "1.x-beta",
  "01.2.3",
  "1.02.3",
  "1.2.3+build",
  "1.2.3-beta+build",
  ">=1.2.3+build",
  "1.2.3-",
  "1.2.3-.1",
  "1.2.3-beta..1",
  "1.2.3-01",
  "1.2.3+",
  "v1.2.3",
  "V1.2.3",
  "== 1.2.3",
  "v=1.2.3",
  "x - 1",
  "1 - x",
  "<1.2.3-0",
  ">=0.0.0-0",
  "~1.2.3-beta.2||^2",
  "^1.2.3-0",
  "1.2.3 - 2.0.0 - 3.0.0",
  ">=1.0.0 - 2.0.0",
  "1 | 2",
  "|",
  "1.2.3 ||| 1",
  "v",
  "=",
  "^*",
  "~*",
  "^x",
  "~1.x",
  ">1.x",
  ">=1.x",
  "<1.x",
  "<=1.x",
  ">1.2.x",
  "<=1.2.x",
  "=1.2",
  "1.X.x",
  "==1.2.3",
  "=v=1.2.3",
  "v1.x",
  "=1.x",
  "==1.x",
  "x.x.x-beta",
  "1.2.*-beta",
  "1.*.3",
  "v=1.x",
  "^v=1.2.3",
  "~=1.2.3",
  ">=v1.2.3",
  ">==1.2.3",
  "<=v=1.2.3",
  "1.2.3 - v2.3.4",
  "v=1 - 2",
  // what must be refused
  "abc",
  "^",
  "~",
  ">=",
  "1..2",
  "1.2.",
  "^^1.2",
  "1.2.3.4",
  "^1.2.3 abc",
  "%",
  "1,2",
  ">=>1",
  "=1.2.3 =",
];

// ------------------------------------------------------------------------------------------------ the oracles
const unique = (list) => [...new Set(list)];

function composerOracle() {
  const php = process.env.PHP ?? "php";
  const phar = process.env.COMPOSER_PHAR;
  if (!phar)
    throw new Error("set COMPOSER_PHAR to the official composer.phar (see the top of this file)");
  const registry = JSON.parse(
    fs.readFileSync(path.join(repo, "registry", "registry.json"), "utf8"),
  );
  const newest = registry.runtimes.composer.versions[0];
  const expected = newest.artifacts[0].sha256;
  const actual = createHash("sha256").update(fs.readFileSync(phar)).digest("hex");
  if (actual !== expected) {
    throw new Error(
      `composer.phar has SHA-256 ${actual}, the registry lists ${expected} for Composer ${newest.version}: not running it`,
    );
  }
  const cases = path.join(os.tmpdir(), `lunarun-composer-cases-${String(process.pid)}.json`);
  fs.writeFileSync(
    cases,
    JSON.stringify({ constraints: unique(composerConstraints), versions: composerVersions }),
  );
  try {
    const answer = execFileSync(
      php,
      ["-d", "phar.readonly=1", path.join(import.meta.dirname, "composer-oracle.php"), phar, cases],
      {
        encoding: "utf8",
        maxBuffer: 64 * 1024 * 1024,
      },
    );
    return { composer: newest.version, versions: composerVersions, cases: JSON.parse(answer) };
  } finally {
    fs.rmSync(cases, { force: true });
  }
}

npmRanges.push(...installedEngines());

function npmOracle() {
  const require = createRequire(import.meta.url);
  const semver = require(process.env.SEMVER_PATH ?? "semver");
  const { version } = require(path.join(process.env.SEMVER_PATH ?? "semver", "package.json"));
  const cases = unique(npmRanges).map((range) => {
    let normalized;
    try {
      normalized = semver.validRange(range);
    } catch {
      normalized = null;
    }
    if (normalized === null) return { constraint: range, valid: false };
    const matches = npmVersions.filter((v) => semver.satisfies(v, range));
    return { constraint: range, valid: true, normalized, matches };
  });
  return { semver: version, versions: npmVersions, cases };
}

/**
 * Every distinct `engines.node` value of the packages installed under node_modules (pnpm keeps them in
 * node_modules/.pnpm/<name>@<version>/node_modules/<name>): real ranges, as real authors wrote them.
 */
function installedEngines() {
  const found = new Set();
  const store = path.join(repo, "node_modules", ".pnpm");
  if (!fs.existsSync(store)) return [];
  const read = (file) => {
    try {
      const engines = JSON.parse(fs.readFileSync(file, "utf8")).engines;
      if (engines && typeof engines.node === "string") found.add(engines.node);
    } catch {
      // not a package.json we can read: skip it
    }
  };
  for (const entry of fs.readdirSync(store)) {
    const modules = path.join(store, entry, "node_modules");
    if (!fs.existsSync(modules)) continue;
    for (const name of fs.readdirSync(modules)) {
      if (name.startsWith("@")) {
        for (const inner of fs.readdirSync(path.join(modules, name)))
          read(path.join(modules, name, inner, "package.json"));
      } else {
        read(path.join(modules, name, "package.json"));
      }
    }
  }
  return [...found].sort();
}

const write = (name, data) => {
  const file = path.join(outDir, name);
  fs.writeFileSync(file, JSON.stringify(data, null, 1) + "\n");
  const valid = data.cases.filter((c) => c.valid).length;
  console.log(
    `${file}: ${String(data.cases.length)} constraints (${String(valid)} accepted) against ${String(data.versions.length)} versions`,
  );
};

write("composer.json", composerOracle());
write("npm.json", npmOracle());
