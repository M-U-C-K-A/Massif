/**
 * Rivers: a field of columns with a valley running down it to the sea at the
 * front. A source lights on a column high in the valley; from it the water
 * finds its way down, column after column: each is dug a little and takes the
 * bright crease of water, until the river reaches the sea.
 */
import { HL, writeFigure, el, g, role, num, animate, animateShow, reD } from "../lib.mjs";
import { stillPrism, risingPrism } from "../solids.mjs";
import { columnKit } from "../field.mjs";

const { Cam, fit, proj, facing, rings, clamp, ringAt, poly, rrect } = HL;

const N = 9, CELL = 14, FOOT = 11, E = N * CELL, PB = 5;

/** The valley's line, from high at the back to the sea at the front, in grid units. */
const line = (s) => [0.6 + 7.6 * s, 2.2 + 4.4 * s + 1.1 * Math.sin(s * Math.PI * 2.2)];

export default function rivers(dir) {
  const W = 400, H = 320, T = 7.4;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [0, 0, 38]], W / 2, H / 2 + 6);
  const P = proj(C), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.6, 0.9);
  const kit = columnKit(P, front, ring, inner, "c");

  // the river's cells, in order down the valley, each touching the last
  const path = [];
  for (let k = 0; k <= 400; k++) {
    const [u, v] = line(k / 400), c = [clamp(Math.round(u), 0, N - 1), clamp(Math.round(v), 0, N - 1)];
    const last = path[path.length - 1];
    if (!last || last[0] !== c[0] || last[1] !== c[1]) path.push(c);
  }
  const onRiver = new Map(path.map((c, k) => [c[0] + "," + c[1], k]));
  const dist = (i, j) => {
    let best = 1e9;
    for (let k = 0; k <= 100; k++) { const [u, v] = line(k / 100); best = Math.min(best, Math.hypot(i - u, j - v)); }
    return best;
  };
  const height = (i, j) => {
    const back = 1 - (i + j) / (2 * (N - 1));
    return 1.2 + 10 * back + (5 + 20 * back) * (1 - Math.exp(-(dist(i, j) ** 2) / 3)) + 1.6 * Math.sin(i * 1.7 + j) ** 2;
  };
  // the bed only ever goes down
  let lastH = Infinity;
  const bed = path.map(([i, j]) => (lastH = Math.min(lastH - 0.9, height(i, j) * 0.6 + 0.8)));

  const tDry = 0.9, tSource = 1.5, tFlow = 2.0, STEP = 0.15;
  const kids = [];
  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const k = onRiver.get(i + "," + j);
    if (k === undefined) { kids.push(kit.column(i * CELL, j * CELL, height(i, j), T)); continue; }
    const wet = Math.max(0.8, bed[k]), dry = wet + 3.2, tk = tFlow + k * STEP;
    const x0 = i * CELL + m, y0 = j * CELL + m;
    const [r, ri] = rings(x0, y0, x0 + FOOT, y0 + FOOT, 2.6, 0.9);
    const keys = [[0, wet, "linear"], [tDry, wet, "lift"], [tDry + 0.7, dry, "linear"], [tk, dry, "lift"], [tk + 0.7, wet, "linear"]];
    const water = [[0, "hi"], [tDry + 0.15, "mid"], [tk + 0.12, "hi"]];
    const col = risingPrism(P, front, r, ri, 0, keys, T, { base: wet });
    // the water: the whole lid's inner ring, bright while the river runs here
    const pool = rrect(x0 + 2.2, y0 + 2.2, x0 + FOOT - 2.2, y0 + FOOT - 2.2, 1.2, 4);
    const poolD = (h) => reD(poly(ringAt(P, pool, h)));
    col.kids.push(el("path", { d: poolD(wet), fill: "none", stroke: role("hi") }, [
      animate("d", keys.map(([t, h, e]) => [t, poolD(h), e]), T),
      animateShow(water.map(([t, r]) => [t, r === "hi"]), T),
    ]));
    // the source: a bright dot on the first column's lid
    if (k === 0) {
      const lid = (h) => P(x0 + FOOT / 2, y0 + FOOT / 2, h);
      const dot = el("ellipse", { cx: num(lid(wet)[0]), cy: num(lid(wet)[1]), rx: 2, ry: 1.2, fill: role("hi"), stroke: "none" }, [
        animate("cy", keys.map(([t, h, e]) => [t, lid(h)[1], e]), T, { fmt: (v) => num(v) }),
        animate("rx", [[0, 2, "linear"], [tDry, 0, "linear"], [tSource, 2, "linear"]], T, { discrete: true }),
      ]);
      kids.push(g({}, [col, dot]));
    } else kids.push(col);
  }
  writeFigure(dir, "rivers", { w: W, h: H, view: [34, 58, 332, 224], title: "A river found from its source down a valley of columns to the sea", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
