/**
 * Roads: a field of hills. A road is laid through it, column after column:
 * the humps are cut and the hollows filled until it climbs no steeper than
 * its slope allows, and its lid takes the bright stroke of the pavement.
 * Then the land comes back as it was.
 */
import { HL, writeFigure, el, g, role, pathOf, animateShow } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";
import { columnKit, sample } from "../field.mjs";

const { Cam, fit, proj, facing, rings, clamp, ringAt, rrect } = HL;

const N = 9, CELL = 14, FOOT = 11, E = N * CELL, PB = 5;
const line = (s) => [0.4 + 8.2 * s, 6.4 - 4.6 * s + 1.4 * Math.sin(s * Math.PI * 1.6)];
const hill = (i, j) => 3 + 24 * Math.sin(i * 0.9 + 0.6) ** 2 * Math.cos(j * 0.55 - 0.4) ** 2 + 12 * Math.exp(-((i - 5) ** 2 + (j - 4) ** 2) / 5) + 2 * Math.sin(i * 2.3 + j * 1.7) ** 2;
const ease = (u) => (u <= 0 ? 0 : u >= 1 ? 1 : 1 - (1 - u) ** 3);

export default function roads(dir) {
  const W = 400, H = 320, T = 7.6;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [0, 0, 34]], W / 2, H / 2 + 6);
  const P = proj(C), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.6, 0.9);
  const kit = columnKit(P, front, ring, inner, "c");

  const path = [];
  for (let k = 0; k <= 400; k++) {
    const [u, v] = line(k / 400), c = [clamp(Math.round(u), 0, N - 1), clamp(Math.round(v), 0, N - 1)];
    const last = path[path.length - 1];
    if (!last || last[0] !== c[0] || last[1] !== c[1]) path.push(c);
  }
  // graded: smoothed, then no step steeper than MAX between neighbours
  const nat = path.map(([i, j]) => hill(i, j));
  let gr = nat.map((_, k) => { let s = 0, n = 0; for (let d = -2; d <= 2; d++) if (nat[k + d] !== undefined) { s += nat[k + d]; n++; } return s / n; });
  const MAX = 1.6;
  for (let pass = 0; pass < 20; pass++) for (let k = 1; k < gr.length; k++) {
    if (gr[k] - gr[k - 1] > MAX) gr[k] = gr[k - 1] + MAX;
    if (gr[k - 1] - gr[k] > MAX) gr[k] = gr[k - 1] - MAX;
  }
  const at = new Map(path.map((c, k) => [c[0] + "," + c[1], k]));
  const t0 = 0.8, STEP = 0.16, tBack = 5.6;
  const pave = rrect(m + 2.2, m + 2.2, m + FOOT - 2.2, m + FOOT - 2.2, 1.2, 4);
  const paveD = pathOf(ringAt(P, pave, 0).map(([x, y]) => [x - P(0, 0, 0)[0] + P(0, 0, 0)[0], y]), true);

  const kids = [];
  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const k = at.get(i + "," + j);
    if (k === undefined) { kids.push(kit.column(i * CELL, j * CELL, hill(i, j), T)); continue; }
    const tk = t0 + k * STEP, h0 = nat[k], h1 = gr[k];
    const track = sample(T, 20, (t) => (t < tBack ? h0 + (h1 - h0) * ease((t - tk) / 0.6) : h1 + (h0 - h1) * ease((t - tBack - k * 0.03) / 0.6)));
    const road = el("path", { d: paveD, fill: "none", stroke: role("hi") }, [animateShow([[0, false], [tk + 0.3, true], [tBack + k * 0.03, false]], T)]);
    kids.push(kit.column(i * CELL, j * CELL, track, T, { extra: [road] }));
  }
  writeFigure(dir, "roads", { w: W, h: H, view: [34, 73, 332, 207], title: "A road graded through a field of hills, its humps cut and its hollows filled", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
