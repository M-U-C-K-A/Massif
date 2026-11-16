/**
 * Selections: a low field. A selection is drawn on it, its marching ants
 * running round; the columns inside rise, and only they, then settle back,
 * and the selection goes.
 */
import { HL, writeFigure, el, g, role, pathOf, animate, animateShow } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";
import { columnKit, sample } from "../field.mjs";

const { Cam, fit, proj, facing, rings } = HL;

const N = 9, CELL = 14, FOOT = 11, E = N * CELL, PB = 5;
const low = (i, j) => 2.4 + 2.2 * Math.sin(i * 1.3 + j * 0.7) ** 2 + 1.2 * Math.cos(j * 1.1) ** 2;
const ease = (u) => (u <= 0 ? 0 : u >= 1 ? 1 : u < 0.5 ? 4 * u ** 3 : 1 - (-2 * u + 2) ** 3 / 2);

export default function select(dir) {
  const W = 400, H = 320, T = 7.2;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [0, 0, 30]], W / 2, H / 2 + 6);
  const P = proj(C), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.6, 0.9);
  const kit = columnKit(P, front, ring, inner, "c");

  const cx = 4.5 * CELL, cy = 4.4 * CELL, rx = 2.9 * CELL, ry = 2.2 * CELL, rot = -0.5;
  const inside = (x, y) => { const dx = x - cx, dy = y - cy, a = dx * Math.cos(rot) + dy * Math.sin(rot), b = -dx * Math.sin(rot) + dy * Math.cos(rot); return (a / rx) ** 2 + (b / ry) ** 2 <= 1; };
  const tAnts = 0.5, tUp = 1.6, tDown = 4.4, tOff = 6.0, RAISE = 16;
  const ell = (z) => { const pts = []; for (let k = 0; k < 64; k++) { const a = (k / 64) * 2 * Math.PI, u = rx * 1.06 * Math.cos(a), v = ry * 1.12 * Math.sin(a); pts.push(P(cx + u * Math.cos(rot) - v * Math.sin(rot), cy + u * Math.sin(rot) + v * Math.cos(rot), z)); } return pathOf(pts, true); };

  const kids = [];
  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const x = (i + 0.5) * CELL, y = (j + 0.5) * CELL, h0 = low(i, j);
    if (!inside(x, y)) { kids.push(kit.column(i * CELL, j * CELL, h0, T)); continue; }
    const d = Math.hypot(x - cx, y - cy) / CELL * 0.06;
    const track = sample(T, 20, (t) => h0 + RAISE * (t < tDown ? ease((t - tUp - d) / 0.8) : 1 - ease((t - tDown - d) / 0.8)));
    kids.push(kit.column(i * CELL, j * CELL, track, T));
  }
  // the ants: on the ground, then lifted with the columns
  const zAt = (t) => 6.2 + RAISE * (t < tDown ? ease((t - tUp) / 0.8) : 1 - ease((t - tDown) / 0.8));
  const zs = sample(T, 10, zAt);
  kids.push(el("path", { d: ell(zs[0][1]), fill: "none", stroke: role("hi"), "stroke-width": "1.2", "stroke-dasharray": "4 3" }, [
    animate("d", zs.map(([t, z]) => [t, ell(z), "linear"]), T),
    animate("stroke-dashoffset", [[0, 0, "linear"], [T, -42, "linear"]], T, { fmt: (v) => String(v) }),
    animateShow([[0, false], [tAnts, true], [tOff, false]], T),
  ]));
  writeFigure(dir, "select", { w: W, h: H, view: [34, 90, 332, 187], title: "A selection drawn on a field: only the columns inside it rise", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
