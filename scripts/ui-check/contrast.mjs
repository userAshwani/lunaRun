// WCAG contrast of the Services page's text in both themes, measured from the colours the browser really computed
// (any CSS colour, including oklch, is converted through a canvas; transparent backgrounds are composited over their
// ancestors). The UI standard wants 4.5:1 for text of this size. Exits 1 if anything is below.
//
//   node scripts/ui-check/contrast.mjs
//
// Needs the dev server and PLAYWRIGHT_CORE (README.md). To check another page, add its scenarios and targets.
import { launch, openPage, runSteps } from "./browser.mjs";
import { SCENARIOS, bridge } from "./services-scenarios.mjs";

/** Per scenario: [label, text on the page whose colours are measured]. */
const TARGETS = {
  data: [
    ["page description", "Start, stop and restart the web server"],
    ["table header", "Service"],
    ["service name", "MySQL 9.7"],
    ["detail line", "Version 2.11.7"],
    ["ports", "9100–9103"],
    ["ports: none yet", "Not assigned yet"],
    ["badge: Running", "Running"],
    ["badge: Starting", "Starting"],
    ["badge: Stopped", "Stopped"],
    ["badge: Stopping", "Stopping"],
    ["badge: Crashed", "Crashed"],
    ["note: pool count", "3 of 4 running"],
    ["note: first start", "Its first start sets up"],
    ["note: error message", "mysqld.exe exited"],
    ["note: gave up", "It crashed again and again"],
    ["note: killed", "It did not stop in time"],
    ["footnote: database", "Databases use the user root"],
  ],
  setup: [
    ["setup note", "Setting up its data folder"],
    ["button: Cancel", "Cancel"],
    ["button: Stopping", "Stopping"],
  ],
  problems: [
    ["problem headline", "MySQL 8.4 did not start"],
    ["problem advice", "Port 3307 is used by"],
    ["problem hint", "That looks like XAMPP"],
    ["problem message", "port 3307 is already in use by"],
    ["problem output", "[ERROR] mariadb-install-db"],
    ["button: Use another port", "Use another port"],
  ],
  logs: [
    ["row name (the page is ready)", "MySQL 8.4"],
    ["panel title", "Logs of MySQL 8.4"],
    ["panel description", "What it prints, live"],
    ["log line", "mysqld: ready for connections"],
    ["log line: lunaRun note", "[lunaRun] ready"],
    ["line count", "lines"],
    ["button: Copy log", "Copy log"],
  ],
  quit: [
    ["row name (the page is ready)", "MySQL 8.4"],
    ["overlay title", "Stopping your services"],
    ["overlay description", "lunaRun closes when they have stopped"],
    ["group name", "Web server"],
    ["group progress", "1 of 1 stopped"],
    ["waiting for", "Waiting for mysql@8.4"],
  ],
  leftovers: [
    ["leftovers title", "Services from an earlier session are still running"],
    ["leftovers item", "MySQL 8.4 (process 4242)"],
    ["leftovers button: Adopt", "Adopt"],
    ["leftovers description", "These were started by an earlier session"],
  ],
  empty: [
    ["empty title", "No services yet"],
    ["empty description", "Services come from the runtimes"],
  ],
  error: [
    ["error title", "Could not load the services"],
    ["error detail", "access is denied"],
  ],
};

/** Runs in the page. Returns a function from a piece of text to the contrast ratio of the text node that holds it. */
const measurer = () => {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  const rgba = (css) => {
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = "#000";
    ctx.fillStyle = css;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
    return [r, g, b, a / 255];
  };
  const over = (top, bottom) => {
    const alpha = top[3] + bottom[3] * (1 - top[3]);
    return [0, 1, 2]
      .map((i) => (top[i] * top[3] + bottom[i] * bottom[3] * (1 - top[3])) / (alpha || 1))
      .concat(alpha);
  };
  const luminance = ([r, g, b]) => {
    const channel = (v) => {
      const c = v / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  };
  const background = (el) => {
    const layers = [];
    for (let node = el; node; node = node.parentElement) {
      layers.push(rgba(getComputedStyle(node).backgroundColor));
    }
    return layers.reduceRight((below, layer) => over(layer, below), [255, 255, 255, 1]);
  };
  return (text) => {
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!node.textContent.includes(text)) continue;
      const el = node.parentElement;
      const bg = background(el);
      const fg = over(rgba(getComputedStyle(el).color), bg);
      const [hi, lo] = [luminance(fg), luminance(bg)].sort((a, b) => b - a);
      return { ratio: (hi + 0.05) / (lo + 0.05), size: getComputedStyle(el).fontSize };
    }
    return null;
  };
};

const NEEDED = 4.5;
const browser = await launch();
let lowest = Infinity;
let failures = 0;
for (const theme of ["light", "dark"]) {
  for (const [name, targets] of Object.entries(TARGETS)) {
    const scenario = { ...SCENARIOS[name], ready: targets[0][1] };
    const { context, page } = await openPage(browser, {
      scenario,
      bridge,
      theme,
      width: 1400,
      height: 900,
      route: "/services",
    });
    await runSteps(page, scenario);
    const results = await page.evaluate(
      ({ source, targets: wanted }) => {
        const measure = new Function(`return (${source})()`)();
        return wanted.map(([label, text]) => [label, measure(text)]);
      },
      { source: measurer.toString(), targets },
    );
    for (const [label, result] of results) {
      if (!result) {
        console.log(`${theme.padEnd(5)} ${label.padEnd(24)} NOT FOUND`);
        failures += 1;
        continue;
      }
      const ok = result.ratio >= NEEDED;
      lowest = Math.min(lowest, result.ratio);
      if (!ok) failures += 1;
      const ratio = result.ratio.toFixed(2).padStart(6);
      console.log(
        `${theme.padEnd(5)} ${label.padEnd(24)} ${ratio}:1 ${result.size.padStart(5)}  ${ok ? "ok" : "BELOW AA"}`,
      );
    }
    await context.close();
  }
}
console.log(`\nlowest ratio: ${lowest.toFixed(2)}:1, failures: ${String(failures)}`);
await browser.close();
process.exit(failures === 0 ? 0 : 1);
