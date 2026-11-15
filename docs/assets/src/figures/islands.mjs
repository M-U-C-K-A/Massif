/**
 * Islands: rock over the void. A field of low columns, the ground, and two
 * islands floating over it, land on their tops and their undersides hanging
 * to points. They bob, slowly and out of step; the larger one's underside has
 * the bright stroke.
 */
import { HL, writeFigure, el, g, animate, role, reD } from "../lib.mjs";
import { stillPrism, taper, rect, drawPair } from "../solids.mjs";
import { columnKit } from "../field.mjs";

const { Cam, fit, proj, facing, rings, prism, ringAt, poly } = HL;

const N = 8, CELL = 15, FOOT = 12, E = N * CELL, PB = 5;

export default function islands(dir) {
  const W = 400, H = 320, T = 6.4;
  const C = Cam(45, 0.5, 1.56);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [34, 30, 110], [60, 66, 82]], W / 2, H / 2 + 4);
  const P = proj(C), front = facing(C);
  const kz = P(0, 0, 0)[1] - P(0, 0, 1)[1];
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.8, 1);
  const kit = columnKit(P, front, ring, inner, "c");
  const kids = [];

  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  // the ground: low, rolling, a little higher at the back
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const u = i / (N - 1), v = j / (N - 1);
    const h = 2 + 7 * (1 - (u + v) / 2) ** 2 + 3 * Math.sin(u * 5.5 + 1) ** 2 * Math.cos(v * 4) ** 2;
    kids.push(kit.column(i * CELL, j * CELL, h, T));
  }

  // the islands: a top slab, land on it, an underside hanging to one or two points
  const isl = [
    { x: 58, y: 64, w: 50, d: 44, z: 62, t: 6, tips: [[0.22, -3, -1, 28], [0.12, 13, 10, 14]], amp: 3.4, phase: 0, bright: true,
      land: [[-12, -8, 11, 10, 12], [0, -10, 10, 10, 7], [-12, 5, 10, 10, 6], [4, 3, 9, 9, 3]] },
    { x: 34, y: 26, w: 28, d: 24, z: 92, t: 5, tips: [[0.22, 2, 1, 18]], amp: 2.4, phase: 0.45, bright: false,
      land: [[-3, -2, 9, 8, 6]] },
  ];
  for (const s of isl.slice().sort((a, b) => a.x + a.y - (b.x + b.y))) {
    const parts = [];
    const [top, tin] = rect(s.x, s.y, s.w, s.d, 9, 1.4);
    // the smaller point first, so the larger one covers it
    for (const [k, dx, dy, dz] of s.tips.slice().sort((a, b) => a[3] - b[3])) {
      const [foot] = rect(s.x + dx, s.y + dy, s.w * k, s.d * k, 9 * k, 0.5);
      const base = k === s.tips[0][0] ? top : rect(s.x + dx, s.y + dy, s.w * 0.42, s.d * 0.42, 6, 0.5)[0];
      parts.push(drawPair(taper(P, front, foot, base, null, s.z - dz, s.z), { stroke: s.bright ? "hi" : "edge" }));
    }
    parts.push(drawPair(prism(P, front, top, tin, s.z, s.z + s.t)));
    for (const [dx, dy, w, d, h] of s.land.slice().sort((a, b) => a[0] + a[1] - (b[0] + b[1]))) {
      const [r2, i2] = rect(s.x + dx, s.y + dy, w, d, 2.4, 0.9);
      parts.push(drawPair(prism(P, front, r2, i2, s.z + s.t, s.z + s.t + h)));
    }
    // a slow bob, out of step with the other island
    const A = s.amp * kz, ph = s.phase * T;
    const keys = [0, 0.25, 0.5, 0.75].map((f, i) => [(f * T + ph) % T, `0 ${[0, -A, 0, A][i].toFixed(1)}`, "inout"]).sort((a, b) => a[0] - b[0]);
    kids.push(g({}, [animate("transform", keys, T, { type: "translate" }), ...parts]));
  }

  writeFigure(dir, "islands", { w: W, h: H, view: [47, 19, 306, 285], title: "Two islands of rock floating over a field of columns", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
