// The directory's carousel screenshots (README.md "Screenshots"): the view
// in the local MCP Apps host, one PNG per prompt, 1000 px wide as the portal
// asks. At device scale 1: in headless Chromium at scale 2 the app lays out
// at the wrong size (the staff takes the ring's room).
//
//   trunk build --release
//   node mcp/local-host.mjs &
//   node docs/claude-directory/screenshots.mjs [out-dir]

import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

// require, not import: it also finds a globally installed Playwright.
const { chromium } = createRequire(import.meta.url)("playwright");

const out = process.argv[2] || path.join(path.dirname(fileURLToPath(import.meta.url)), "screenshots");

// The tool's arguments as Claude chooses them for each prompt in README.md.
const shots = [
  { file: "1-four-four.png", args: "bpm=90&ts=4/4" },
  { file: "2-seven-eight.png", args: "bpm=70&ts=7/8&groups=2%2B2%2B3" },
  { file: "3-polyrhythm.png", args: "bpm=80&ts=4/4&poly=3:2" },
  { file: "4-swing.png", args: "bpm=120&ts=4/4&subdivision=e&swing=66" },
  { file: "5-fullscreen-dark.png", args: "bpm=70&ts=7/8&groups=2%2B2%2B3", theme: "dark", fullscreen: true },
];

const browser = await chromium.launch();
for (const shot of shots) {
  const page = await browser.newPage({
    viewport: { width: 1100, height: 900 },
    colorScheme: shot.theme || "light",
  });
  // Fullscreen without the stand-in composer, so without its room below.
  const insets = shot.fullscreen ? "&insets=0" : "";
  await page.goto(`http://localhost:8080/?wasm=1&w=1000${insets}&theme=${shot.theme || "light"}&${shot.args}`);
  const view = page.frameLocator("#view");
  // The app draws into its canvas once the skeleton is gone.
  await view.locator("canvas").waitFor();
  await view.locator("#loading_text").waitFor({ state: "hidden" });
  if (shot.fullscreen) {
    await page.evaluate(() => window.enterFullscreen());
    await page.evaluate(() => (document.getElementById("composer").style.display = "none"));
  }
  await page.waitForTimeout(3000);
  // The view alone, without the frame's border.
  const box = await page.locator("#view").boundingBox();
  const clip = { x: box.x + 1, y: box.y + 1, width: box.width - 2, height: box.height - 2 };
  await page.screenshot({ path: path.join(out, shot.file), clip });
  console.log(shot.file);
  await page.close();
}
await browser.close();
