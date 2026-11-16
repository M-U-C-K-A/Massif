/**
 * Real places: a flat field. A line of bright columns sweeps it from the back
 * to the front, and behind it the land rises into the relief of a real
 * range: peaks, a valley, a lake's hollow. Then it lies flat again.
 */
import { HL, writeFigure, el, g } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";
import { columnKit, sample } from "../field.mjs";

const { Cam, fit, proj, facing, rings } = HL;

const N = 10, CELL = 13, FOOT = 10.4, E = N * CELL, PB = 5;
const peak = (u, v, a, b, h, w) => h * Math.exp(-((u - a) ** 2 + (v - b) ** 2) / w);
const real = (i, j) => {
  const u = i / (N - 1), v = j / (N - 1);
  return 2 + peak(u, v, 0.32, 0.3, 40, 0.05) + peak(u, v, 0.72, 0.22, 26, 0.035) + peak(u, v, 0.62, 0.72, 16, 0.04) + 4 * Math.sin(u * 9 + v * 5) ** 2 * (1 - v) - 6 * peak(u, v, 0.2, 0.82, 1, 0.02);
};
const ease = (u) => (u <= 0 ? 0 : u >= 1 ? 1 : 1 - (1 - u) ** 3);

export default function relief(dir) {
  const W = 400, H = 320, T = 7.4;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [0, 0, 40]], W / 2, H / 2 + 6);
  const P = proj(C), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.4, 0.85);
  const kit = columnKit(P, front, ring, inner, "c");

  const t0 = 0.7, STEP = 0.17, tFlat = 5.4;
  const kids = [];
  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const h1 = Math.max(1, real(i, j)), tk = t0 + s * STEP;
    const track = sample(T, 12, (t) => (t < tFlat ? 1 + (h1 - 1) * ease((t - tk) / 0.7) : h1 + (1 - h1) * ease((t - tFlat - s * 0.04) / 0.6)));
    const roles = [[0, "edge"], [tk, "hi"], [tk + STEP * 1.6, "edge"]];
    kids.push(kit.column(i * CELL, j * CELL, track, T, { stroke: roles }));
  }
  writeFigure(dir, "relief", { w: W, h: H, view: [29, 82, 341, 204], title: "A flat field rising, row after row, into the relief of a real range", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
