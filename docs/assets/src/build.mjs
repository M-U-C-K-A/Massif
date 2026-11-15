/**
 * Draws the README's figures: `node docs/assets/src/build.mjs [out] [names…]`
 * writes `<name>-light.svg` and `<name>-dark.svg` for each figure into `out`
 * (by default docs/assets, the folder above this one).
 */
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const [outArg, ...only] = process.argv.slice(2);
const out = outArg ? path.resolve(outArg) : path.resolve(here, "..");
const names = ["mark", "brush", "islands", "graph", "rivers", "blocks"];
for (const n of names) {
  if (only.length && !only.includes(n)) continue;
  (await import(`./figures/${n}.mjs`)).default(out);
}
