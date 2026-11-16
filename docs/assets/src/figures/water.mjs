/**
 * Water: a river coming down a valley towards us in steps, as the game keeps
 * it. From the source the pools fill one after another, the last cell of each
 * tilted where the water starts to flow; over every step the water falls in a
 * sheet into the pool below. Then it holds: the sheets run, nothing spills.
 */
import { HL, writeFigure, el, g, role, num, pathOf, animate, animateShow, boxed } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";
import { columnKit } from "../field.mjs";

const { Cam, fit, proj, facing, rings, rrect } = HL;

const N = 7, CELL = 14, FOOT = 11, E = N * CELL, PB = 5, INSET = 1.5;

/** The river, pool after pool down the valley's middle, from the back corner to the front one. */
const POOLS = [
  { level: 15.5, cells: [[0, 0], [1, 0], [1, 1]] },
  { level: 11.5, cells: [[2, 1], [2, 2], [3, 2]] },
  { level: 7.5, cells: [[3, 3], [4, 3], [4, 4]] },
  { level: 3.2, cells: [[5, 4], [5, 5], [6, 5], [6, 6]] },
];

export default function water(dir) {
  const W = 400, H = 320, T = 7.6;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [0, 0, 30]], W / 2, H / 2 + 6);
  const P = boxed(proj(C)), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.6, 0.9);
  const kit = columnKit(P, front, ring, inner, "c");

  const river = new Map();
  POOLS.forEach((p, k) => p.cells.forEach(([i, j], n) => {
    const next = n === p.cells.length - 1 && k < POOLS.length - 1 ? POOLS[k + 1].cells[0] : null;
    river.set(i + "," + j, { pool: k, n, step: next && [next[0] - i, next[1] - j] });
  }));
  const wet = POOLS.flatMap((p) => p.cells.map(([i, j]) => ({ i, j, level: p.level })));
  // the valley's sides climb away from the river, from the height of its nearest water
  const height = (i, j) => {
    let near = null, d = 1e9;
    for (const c of wet) { const e = Math.hypot(i - c.i, j - c.j); if (e < d) { d = e; near = c; } }
    return near.level + 2.2 + 3.4 * (d - 1) + 1.3 * Math.sin(i * 2.1 + j * 1.4) ** 2;
  };
  const bedOf = (level) => Math.max(0.8, level - 1.2);

  // the times: the source, each pool filling, the hold, then all of it gone
  const tSource = 0.45, tFirst = 0.8, POOL = 0.95, CELLSTEP = 0.12, RISE = 0.5, tEnd = 6.8, DRAIN = 0.45;
  const tFill = (k, n) => tFirst + k * POOL + n * CELLSTEP;

  // a cell's water: its lid's inner ring at the water's height, tilted down at the lip of a step
  const waterRing = (i, j, z, step, tilt) => {
    const x0 = i * CELL + m + INSET, y0 = j * CELL + m + INSET, x1 = (i + 1) * CELL - m - INSET, y1 = (j + 1) * CELL - m - INSET;
    const drop = (q) => (!step || !tilt ? 0 : step[0] ? tilt * (q.u - x0) / (x1 - x0) : tilt * (q.v - y0) / (y1 - y0));
    return pathOf(rrect(x0, y0, x1, y1, 1.2, 4).map((q) => P(q.u, q.v, z - drop(q))), true);
  };
  const kids = [];
  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const r = river.get(i + "," + j);
    if (!r) {
      const h = height(i, j);
      for (const [u, v] of [[0, 0], [1, 0], [0, 1], [1, 1]]) P((i + u) * CELL, (j + v) * CELL, h);
      kids.push(kit.column(i * CELL, j * CELL, h, T));
      continue;
    }
    const { level } = POOLS[r.pool], bed = bedOf(level), t0 = tFill(r.pool, r.n), tilt = r.step ? 1.2 : 0;
    kids.push(kit.column(i * CELL, j * CELL, bed, T));
    const low = waterRing(i, j, bed + 0.3, null, 0), full = waterRing(i, j, level, r.step, tilt);
    kids.push(el("path", { d: low, fill: "none", stroke: role("aqua") }, [
      animate("d", [[0, low, "linear"], [t0, low, "out"], [t0 + RISE, full, "linear"], [tEnd, full, "in"], [tEnd + DRAIN, low, "linear"]], T),
      animateShow([[0, false], [t0, true], [tEnd + DRAIN, false]], T),
    ]));
    // the source: a bright dot in the first pool
    if (r.pool === 0 && r.n === 0) {
      const at = P((i + 0.5) * CELL, (j + 0.5) * CELL, level);
      kids.push(el("ellipse", { cx: num(at[0]), cy: num(at[1]), rx: 1.9, ry: 1.1, fill: role("aqua"), stroke: "none" }, [animateShow([[0, false], [tSource, true], [tEnd + DRAIN, false]], T)]));
    }
    // over the step: a sheet of falling water, from the lip down to the next pool
    if (r.step) {
      const below = POOLS[r.pool + 1].level, top = level - tilt, tOn = t0 + RISE;
      const lip = r.step[0] ? (d) => [(i + 1) * CELL - m - 0.3, j * CELL + m + d] : (d) => [i * CELL + m + d, (j + 1) * CELL - m - 0.3];
      const lines = [2.8, 5.5, 8.2].map((d) => { const [x, y] = lip(d); return [P(x, y, top), P(x, y, below)]; });
      kids.push(g({ fill: "none", stroke: role("aqua"), "stroke-dasharray": "2.4 2.2" }, [
        animateShow([[0, false], [tOn, true], [tEnd, false]], T),
        el("animate", { attributeName: "stroke-dashoffset", values: "0;-9.2", dur: "0.9s", repeatCount: "indefinite" }),
        ...lines.map((pts) => el("path", { d: pathOf(pts) })),
      ]));
    }
  }
  for (const [x, y] of [[-6, -6], [E + 6, E + 6], [E + 6, -6], [-6, E + 6]]) P(x, y, -PB);
  writeFigure(dir, "water", { w: W, h: H, view: P.view(5), title: "A river comes down a valley in steps: the pools fill from the source, the water falls over each step, and it holds", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
