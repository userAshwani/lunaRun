// Makes a throw-away LUNA_HOME with a few runtimes "installed" (a receipt and a stand-in program each), so that the real
// app window has something to show without downloading anything and without touching your real data folder. The folder
// name has a space and a non-ASCII letter on purpose (NFR-07).
//
//   node scripts/ui-check/seed-home.mjs <parent folder>     # creates <parent>/lüna home and prints it
//
// Then run real-window.ps1 with that parent (README.md).
import fs from "node:fs";
import path from "node:path";

const parent = process.argv[2];
if (!parent) {
  console.error("usage: node scripts/ui-check/seed-home.mjs <parent folder>");
  process.exit(2);
}
const home = path.join(path.resolve(parent), "lüna home");
fs.rmSync(home, { recursive: true, force: true });

const receipt = (runtime, version) => {
  const dir = path.join(home, "runtimes", runtime, version);
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
  fs.writeFileSync(path.join(dir, "bin", "program"), "stand-in");
  const os = { win32: "windows", darwin: "macos", linux: "linux" }[process.platform] ?? "windows";
  fs.writeFileSync(
    path.join(dir, "install.json"),
    JSON.stringify({
      schema: 1,
      runtime,
      version,
      os,
      arch: "x86_64",
      rosetta: false,
      installedAt: 1767225600,
      artifacts: [],
    }),
  );
};
for (const [runtime, version] of [
  ["caddy", "2.11.7"],
  ["php", "8.3.36"],
  ["php", "8.4.12"],
  ["mysql", "8.4.13"],
  ["mysql", "8.4.6"], // an older patch of the same line: the list shows the line once, from 8.4.13
  ["mariadb", "12.3.3"],
]) {
  receipt(runtime, version);
}

// MariaDB already has data; MySQL has none, so its row says its first start sets it up.
fs.mkdirSync(path.join(home, "data", "mariadb-12.3"), { recursive: true });
fs.writeFileSync(path.join(home, "data", "mariadb-12.3", "ibdata1"), "data");

// Ports given at an earlier first start.
fs.mkdirSync(path.join(home, "config"), { recursive: true });
fs.writeFileSync(
  path.join(home, "config", "services.json"),
  JSON.stringify({
    schema: 1,
    ports: {
      "mysql@8.4": { port: 3307, count: 1 },
      "php-fcgi@8.3.36": { port: 9100, count: 4 },
      caddy: { port: 2119, count: 1 },
    },
  }),
);
console.log(home);
