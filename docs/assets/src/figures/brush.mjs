/**
 * Brush: a field of columns, a massif at rest. A brush crosses it in one
 * stroke and raises a ridge where it passes, the more the nearer its centre;
 * the column under the centre takes the bright stroke. Then the stroke is
 * undone, from where it ended outwards, and the massif is as it was.
 */
import { HL, writeFigure, el, g } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";
import { columnKit } from "../field.mjs";

const { Cam, fit, proj, facing, rings, spring, stepS, clamp } = HL;

const N = 9, CELL = 14, FOOT = 11, EXT = N * CELL, PB = 5, HMAX = 56;

/** Share of the brush's strength at u radii from its centre. */
const falloff = (u) => (u <= 0.42 ? 1 - (u / 0.42) * 0.69 : u <= 1 ? 0.31 - ((u - 0.42) / 0.58) * 0.31 : 0);

export function restHeight(i, j) {
  const u = i / (N - 1), v = j / (N - 1);
  return 2.5 + 36 * Math.exp(-((u - 0.3) ** 2 + (v - 0.26) ** 2) / 0.03) + 19 * Math.exp(-((u - 0.12) ** 2 + (v - 0.62) ** 2) / 0.03)
    + 8 * Math.exp(-((u - 0.58) ** 2 + (v - 0.1) ** 2) / 0.02) + 1.5 * Math.sin(u * 9 + v * 4) ** 2;
}

export default function brush(dir) {
  const W = 400, H = 320, T = 7.8;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [EXT + 6, EXT + 6, -PB], [EXT + 6, -6, -PB], [-6, EXT + 6, -PB], [0, 0, HMAX * 0.8]], W / 2, H / 2 + 4);
  const P = proj(C), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.6, 0.9);
  const kit = columnKit(P, front, ring, inner, "c");

  // the stroke: across the near half of the field, screen left to right, bending
  const t0 = 0.6, t1 = 4.2, tu = 5.3, L = 250, A = 16, OFF = 22, R = 2.5 * CELL, RATE = 2.4, RAISE = 19, STEP = 0.05;
  const brushAt = (t) => {
    const s = clamp((t - t0) / (t1 - t0), 0, 1);
    const a = (s - 0.5) * L, b = OFF + A * Math.sin(2 * Math.PI * s);
    return [EXT / 2 + (a + b) / Math.SQRT2, EXT / 2 + (-a + b) / Math.SQRT2];
  };
  const onField = (p) => p[0] >= 0 && p[0] < EXT && p[1] >= 0 && p[1] < EXT;
  let lastOn = null;
  for (let t = t0; t <= t1; t += 0.01) { const q = brushAt(t); if (onField(q)) lastOn = q; }

  const cols = [];
  for (let i = 0; i < N; i++) for (let j = 0; j < N; j++) {
    const h0 = restHeight(i, j);
    cols.push({ i, j, cx: (i + 0.5) * CELL, cy: (j + 0.5) * CELL, h0, g: 0, sp: spring(h0), track: [], roles: [] });
  }
  const peak = cols.reduce((a, b) => (b.h0 > a.h0 ? b : a));

  const dt = 1 / 120, every = 6; // simulate at 120 Hz, keep 20 Hz
  for (let k = 0, t = 0; t <= T + 1e-9; k++, t = k * dt) {
    const p = brushAt(t), stroking = t >= t0 && t <= t1, on = stroking && onField(p);
    const under = on ? cols.find((c) => c.i === Math.floor(p[0] / CELL) && c.j === Math.floor(p[1] / CELL)) : null;
    for (const c of cols) {
      if (stroking) c.g = Math.min(1, c.g + RATE * falloff(Math.hypot(c.cx - p[0], c.cy - p[1]) / R) * dt);
      const undone = t >= tu + (Math.hypot(c.cx - lastOn[0], c.cy - lastOn[1]) / CELL) * STEP;
      c.sp.t = Math.min(HMAX, undone ? c.h0 : c.h0 + RAISE * c.g * (2 - c.g));
      stepS(c.sp, dt);
      if (k % every === 0) {
        c.track.push([t, c.sp.x]);
        const r = under ? (under === c ? "hi" : "edge") : c === peak && (t < t0 || t > tu + 0.8) ? "hi" : "edge";
        if (!c.roles.length || c.roles[c.roles.length - 1][1] !== r) c.roles.push([t, r]);
      }
    }
  }

  const kids = [];
  const [pr, pi] = rings(-6, -6, EXT + 6, EXT + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  // diagonal by diagonal from the far corner: painting back to front
  for (let s = 0; s <= 2 * (N - 1); s++) for (const c of cols.filter((c) => c.i + c.j === s)) {
    kids.push(kit.column(c.i * CELL, c.j * CELL, c.track, T, { stroke: c.roles.length > 1 ? c.roles : c.roles[0][1] }));
  }
  writeFigure(dir, "brush", { w: W, h: H, view: [34, 88, 332, 197], title: "A brush raising a ridge across a massif, then the stroke undone", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
