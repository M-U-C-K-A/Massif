/**
 * A contact sheet of one figure: both themes, paused at chosen times (inline
 * SVG, setCurrentTime), taken by a headless Chrome through playwright-core.
 * usage: node docs/assets/src/sheet.mjs <name> [t0,t1,…] [scale]
 * It reads docs/assets/<name>-{light,dark}.svg and writes the sheet into the
 * system's temporary folder. playwright-core is looked for in the cache the
 * Hairline skill's look.mjs installs it to, or HAIRLINE_LOOK_CACHE.
 */
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import os from "node:os";
const cache = process.env.HAIRLINE_LOOK_CACHE || path.join(os.homedir(), process.platform === "darwin" ? "Library/Caches/hairline-look" : ".cache/hairline-look");
const req = createRequire(path.join(cache, "/"));
const { chromium } = req("playwright-core");

const [name, ts = "0,0.5,1,2", sc = "1"] = process.argv.slice(2);
const times = ts.split(",").map(Number), scale = Number(sc);
const svgs = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = fs.mkdtempSync(path.join(os.tmpdir(), "massif-sheet-"));
const bg = { light: "#ffffff", dark: "#0d1117" };
let html = `<!doctype html><meta charset=utf-8><style>body{margin:0;font:11px -apple-system,sans-serif;background:#ddd}
.row{display:flex;gap:0;width:max-content}.c{padding:10px;position:relative}.c span{position:absolute;left:6px;top:4px;color:#888}
svg{display:block}</style>`;
for (const theme of ["light", "dark"]) {
  const svg = fs.readFileSync(path.join(svgs, `${name}-${theme}.svg`), "utf8");
  const w = Number(/width="([\d.]+)"/.exec(svg)[1]) * scale, h = Number(/height="([\d.]+)"/.exec(svg)[1]) * scale;
  html += `<div class=row>`;
  times.forEach((t, k) => { const pre = `${theme[0]}${k}-`; const s = svg.replace(/width="[\d.]+" height="[\d.]+"/, `width="${w}" height="${h}" data-t="${t}"`).replace(/ id="/g, ` id="${pre}`).replace(/href="#/g, `href="#${pre}`); html += `<div class=c style="background:${bg[theme]}"><span>${theme} t=${t}</span>${s}</div>`; });
  html += `</div>`;
}
html += `<script>for (const s of document.querySelectorAll("svg[data-t]")) { s.pauseAnimations(); s.setCurrentTime(Number(s.dataset.t)); }</script>`;
const file = path.join(out, `sheet-${name}.html`);
fs.writeFileSync(file, html);
const browser = await chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
const page = await browser.newPage({ deviceScaleFactor: 2, viewport: { width: 2400, height: 1600 } });
const errors = [];
page.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errors.push(m.text()); });
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto("file://" + file);
await page.waitForTimeout(300);
await page.evaluate(() => { for (const s of document.querySelectorAll("svg[data-t]")) { s.pauseAnimations(); s.setCurrentTime(Number(s.dataset.t)); } });
await page.waitForTimeout(200);
const png = path.join(out, `sheet-${name}.png`);
const area = await page.evaluate(() => { const rows = [...document.querySelectorAll(".row")]; return { w: Math.max(...rows.map((x) => x.scrollWidth)), h: rows.reduce((a, x) => a + x.getBoundingClientRect().height, 0) }; });
await page.screenshot({ path: png, clip: { x: 0, y: 0, width: Math.ceil(area.w), height: Math.ceil(area.h) } });
// the drawing's box over the whole loop, in viewBox units: what `view` should hold, with a margin
const box = await page.evaluate(async () => {
  const s = document.querySelector("svg[data-t]");
  const dur = Math.max(...[...s.querySelectorAll("animate,animateTransform,animateMotion")].map((a) => parseFloat(a.getAttribute("dur")) || 0), 0);
  const vb = s.viewBox.baseVal;
  let b = null;
  for (let t = 0; t <= dur + 1e-6; t += 0.1) {
    s.pauseAnimations(); s.setCurrentTime(t);
    await new Promise((r) => requestAnimationFrame(r));
    const sb = s.getBoundingClientRect(), k = vb.width / sb.width;
    for (const el of s.querySelectorAll("path,use,ellipse,circle")) {
      const e = el.getBoundingClientRect();
      if (!e.width && !e.height) continue;
      const q = [vb.x + (e.left - sb.left) * k, vb.y + (e.top - sb.top) * k, vb.x + (e.right - sb.left) * k, vb.y + (e.bottom - sb.top) * k];
      b = b ? [Math.min(b[0], q[0]), Math.min(b[1], q[1]), Math.max(b[2], q[2]), Math.max(b[3], q[3])] : q;
    }
  }
  return b.map((v) => Math.round(v * 10) / 10);
});
await browser.close();
console.log(png, errors.length ? "ERRORS: " + errors.join(" | ") : "no console errors");
console.log(`box over the loop: ${box.join(", ")} (x0, y0, x1, y1)`);
