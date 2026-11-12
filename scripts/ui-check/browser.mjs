// Shared by services-shots.mjs and contrast.mjs: start headless Edge with the settings that worked on a slow Windows PC
// (see README.md), and the address and theme handling.
import path from "node:path";
import { pathToFileURL } from "node:url";

export const BASE_URL = process.env.UI_URL ?? "http://127.0.0.1:1420";
export const OUT_DIR = path.resolve(process.env.OUT_DIR ?? path.join(import.meta.dirname, "out"));
const EDGE =
  process.env.EDGE_PATH ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";

/** `playwright-core` is not a dependency of the repo: point `PLAYWRIGHT_CORE` at the one in your scratch folder. */
export async function launch() {
  const location = process.env.PLAYWRIGHT_CORE;
  if (!location) {
    throw new Error(
      "Set PLAYWRIGHT_CORE to <scratch folder>/node_modules/playwright-core/index.mjs (see scripts/ui-check/README.md).",
    );
  }
  const { chromium } = await import(pathToFileURL(path.resolve(location)).href);
  return chromium.launch({
    executablePath: EDGE,
    headless: true,
    args: ["--no-proxy-server", "--disable-background-networking", "--no-first-run"],
  });
}

/** A page of the app in `theme` ("light" or "dark") with the fake backend, on `route` (`/services`). */
export async function openPage(browser, { scenario, bridge, theme, width, height, route }) {
  const context = await browser.newContext({ viewport: { width, height } });
  const page = await context.newPage();
  await page.addInitScript(bridge(scenario));
  await page.addInitScript(`localStorage.setItem("lunarun.theme", "${theme}")`);
  // The first load against a cold dev server can take half a minute on a slow PC.
  await page.goto(`${BASE_URL}/#${route}`, { waitUntil: "domcontentloaded", timeout: 240_000 });
  await page.getByText(scenario.ready, { exact: false }).first().waitFor({ timeout: 120_000 });
  await page.waitForTimeout(300);
  return { context, page };
}
