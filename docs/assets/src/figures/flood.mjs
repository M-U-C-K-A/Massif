/**
 * Flood: a hollow in a field, its rim lowest at one notch. A click in the
 * hollow, and the water rises in it, cell after cell going under, up to the
 * notch, where it would spill: there it stops, and the notch lights. Then the
 * water goes down again.
 */
import { HL, writeFigure, el, g, role, num, pathOf, animate, animateShow, animateRole, boxed, EASE } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";
import { columnKit } from "../field.mjs";

const { Cam, fit, proj, facing, rings, rrect } = HL;

const N = 9, CELL = 14, FOOT = 11, E = N * CELL, PB = 5;
const CX = 4, CY = 4, R = 3.1;

/** The rim: tall at the back, low at the front so the hollow shows, and lowest at a notch on the right. */
const rim = (a) => 13 - 4.8 * Math.cos(a - Math.PI / 4) - 3.2 * Math.exp(-((Math.atan2(Math.sin(a - 0.12), Math.cos(a - 0.12))) ** 2) / 0.12);
const ground = (i, j) => {
  const di = i - CX, dj = j - CY, d = Math.hypot(di, dj), a = Math.atan2(dj, di), top = rim(a);
  if (d < R) return 1.2 + (top - 1.2) * (d / R) ** 2.4;
  return top + 1.5 * (d - R) + 1.1 * Math.sin(i * 1.7 + j * 2.3) ** 2;
};

/** The cubic-bezier easing of a key spline, as a function of time in [0, 1]. */
function eased(spline) {
  const [x1, y1, x2, y2] = spline.split(" ").map(Number);
  const bz = (a, b, t) => 3 * a * t * (1 - t) ** 2 + 3 * b * t * t * (1 - t) + t ** 3;
  return (u) => {
    if (u <= 0) return 0; if (u >= 1) return 1;
    let lo = 0, hi = 1;
    for (let k = 0; k < 40; k++) { const mid = (lo + hi) / 2; if (bz(x1, x2, mid) < u) lo = mid; else hi = mid; }
    return bz(y1, y2, (lo + hi) / 2);
  };
}

export default function flood(dir) {
  const W = 400, H = 320, T = 7.4;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[-6, -6, -PB], [E + 6, E + 6, -PB], [E + 6, -6, -PB], [-6, E + 6, -PB], [0, 0, 28]], W / 2, H / 2 + 6);
  const P = boxed(proj(C)), front = facing(C);
  const m = (CELL - FOOT) / 2;
  const [ring, inner] = rings(m, m, m + FOOT, m + FOOT, 2.6, 0.9);
  const kit = columnKit(P, front, ring, inner, "c");

  const h = [];
  for (let i = 0; i < N; i++) { h.push([]); for (let j = 0; j < N; j++) h[i].push(ground(i, j)); }
  // where the water would spill: the lowest way out of the hollow, found as Massif finds it,
  // by flooding from the click and always climbing over the lowest cell of the shore
  const key = (i, j) => i * N + j, seen = new Set([key(CX, CY)]), shore = [];
  const push = (i, j) => { if (i >= 0 && j >= 0 && i < N && j < N && !seen.has(key(i, j))) { seen.add(key(i, j)); shore.push([i, j]); } };
  const around = (i, j) => { push(i + 1, j); push(i - 1, j); push(i, j + 1); push(i, j - 1); };
  around(CX, CY);
  let level = h[CX][CY], pass = null;
  for (;;) {
    shore.sort((a, b) => h[a[0]][a[1]] - h[b[0]][b[1]]);
    const [i, j] = shore.shift();
    if (h[i][j] > level) { level = h[i][j]; pass = [i, j]; }
    if (i === 0 || j === 0 || i === N - 1 || j === N - 1) break;
    around(i, j);
  }
  const floor = h[CX][CY];

  // the times: the click, the rise, the notch lit, the hold, the fall
  const tClick = 0.5, tRise = 0.9, tFull = 3.5, tDrain = 6.1, tDry = 6.9;
  const up = eased(EASE.inout), zAt = (t) => (t < tRise ? floor : t < tFull ? floor + (level - floor) * up((t - tRise) / (tFull - tRise)) : t < tDrain ? level : t < tDry ? level - (level - floor) * up((t - tDrain) / (tDry - tDrain)) : floor);
  const crossing = (z) => {
    // when the water passes height z on the way up and on the way down
    let on = null, off = null;
    for (let t = 0; t <= T; t += 0.01) { const w = zAt(t); if (on === null && t >= tRise && w > z) on = t; if (on !== null && off === null && t >= tDrain && w <= z) off = t; }
    return [on, off ?? tDry];
  };
  const waterAt = (i, j, z) => pathOf(rrect(i * CELL + 0.7, j * CELL + 0.7, (i + 1) * CELL - 0.7, (j + 1) * CELL - 0.7, 1.6, 4).map((q) => P(q.u, q.v, z)), true);

  const kids = [];
  const [pr, pi] = rings(-6, -6, E + 6, E + 6, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0));
  for (let s = 0; s <= 2 * (N - 1); s++) for (let i = 0; i < N; i++) {
    const j = s - i;
    if (j < 0 || j >= N) continue;
    const hij = h[i][j];
    for (const [u, v] of [[0, 0], [1, 0], [0, 1], [1, 1]]) P((i + u) * CELL, (j + v) * CELL, hij);
    const isPass = pass && pass[0] === i && pass[1] === j;
    kids.push(kit.column(i * CELL, j * CELL, hij, T, isPass ? { stroke: [[0, "edge"], [tFull - 0.05, "aqua"], [tDrain, "edge"]] } : {}));
    // the water over this cell, from the moment it goes under
    const under = Math.hypot(i - CX, j - CY) < R + 0.5 && hij < level - 0.25;
    if (!under) continue;
    const [on, off] = crossing(hij + 0.25);
    if (on === null) continue;
    kids.push(el("path", { d: waterAt(i, j, floor), fill: "none", stroke: role("aqua") }, [
      animate("d", [[0, waterAt(i, j, floor), "linear"], [tRise, waterAt(i, j, floor), EASE.inout], [tFull, waterAt(i, j, level), "linear"], [tDrain, waterAt(i, j, level), EASE.inout], [tDry, waterAt(i, j, floor), "linear"]], T),
      animateShow([[0, false], [on, true], [off, false]], T),
    ]));
    // the click: a dot on the hollow's floor and a ring going out from it
    if (i === CX && j === CY) {
      const at = P((i + 0.5) * CELL, (j + 0.5) * CELL, floor);
      kids.push(el("ellipse", { cx: num(at[0]), cy: num(at[1]), rx: 1.9, ry: 1.1, fill: role("aqua"), stroke: "none" }, [animateShow([[0, false], [tClick, true], [tDry, false]], T)]));
      kids.push(el("ellipse", { cx: num(at[0]), cy: num(at[1]), rx: 2, ry: 1.2, fill: "none", stroke: role("aqua") }, [
        animate("rx", [[0, 2, "linear"], [tClick, 2, "out"], [tClick + 0.7, 9, "linear"]], T, { once: false }),
        animate("ry", [[0, 1.2, "linear"], [tClick, 1.2, "out"], [tClick + 0.7, 5.2, "linear"]], T),
        animateShow([[0, false], [tClick, true], [tClick + 0.7, false]], T),
      ]));
    }
  }
  for (const [x, y] of [[-6, -6], [E + 6, E + 6], [E + 6, -6], [-6, E + 6]]) P(x, y, -PB);
  writeFigure(dir, "flood", { w: W, h: H, view: P.view(5), title: "A click in a hollow: the water rises up to the lowest point of its rim, where it would spill", body: [el("defs", {}, kit.defs), g({}, kids)] });
}
