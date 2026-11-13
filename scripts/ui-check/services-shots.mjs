// Screenshots of the Services page in every state, in both themes, at 1400 wide and at the app's minimum window size
// (900x600), against the running Vite dev server with a fake Tauri backend. Look at the images afterwards.
//
//   node scripts/ui-check/services-shots.mjs            # every state
//   node scripts/ui-check/services-shots.mjs data empty  # only these
//
// Needs the dev server (`pnpm exec vite --host 127.0.0.1 --port 1420 --strictPort`) and PLAYWRIGHT_CORE (README.md).
import fs from "node:fs";
import path from "node:path";
import { OUT_DIR, launch, openPage, runSteps } from "./browser.mjs";
import { SCENARIOS, bridge } from "./services-scenarios.mjs";

const only = process.argv.slice(2);
fs.mkdirSync(OUT_DIR, { recursive: true });
const browser = await launch();
for (const [name, scenario] of Object.entries(SCENARIOS)) {
  if (only.length > 0 && !only.includes(name)) continue;
  for (const theme of ["light", "dark"]) {
    for (const [width, height] of [
      [1400, 900],
      [900, 600],
    ]) {
      const { context, page } = await openPage(browser, {
        scenario,
        bridge,
        theme,
        width,
        height,
        route: "/services",
      });
      await runSteps(page, scenario);
      await page.waitForTimeout(300);
      const file = path.join(OUT_DIR, `services-${name}-${theme}-${String(width)}.png`);
      await page.screenshot({ path: file, fullPage: true });
      console.log("saved", file);
      await context.close();
    }
  }
}
await browser.close();
