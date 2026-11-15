/**
 * Fields of columns that rise and fall. Every column of a field has the same
 * footprint, so its parts are drawn once in <defs> and each column is a group
 * of <use>s moved into place. A column's height moves two things only: its top
 * (a translation) and its sides (a vertical scale), so an animation is a list
 * of numbers rather than of paths.
 *
 * The parts, for a convex footprint seen from the 2:1 camera: the bottom ring's
 * front run, the two vertical sides at the ring's left and right extremes, the
 * top ring's back run, and the crease. Filled, the bottom ring, the band
 * between the sides and the top ring make up the silhouette exactly.
 */
import { HL, el, g, pathOf, num, role, animate, animateRole, simplify } from "./lib.mjs";

const { ringAt, run } = HL;

export function columnKit(P, front, ring, inner, id) {
  const B = ringAt(P, ring, 0);
  let iL = 0, iR = 0;
  B.forEach((p, i) => { if (p[0] < B[iL][0]) iL = i; if (p[0] > B[iR][0]) iR = i; });
  if (Math.abs(B[iL][1] - B[iR][1]) > 0.05) throw new Error("columnKit: the footprint must be symmetric on screen");
  const walk = (a, b, step) => { const out = []; for (let i = a; ; i = (i + step + B.length) % B.length) { out.push(B[i]); if (i === b) break; } return out; };
  const fwd = walk(iL, iR, 1), back = walk(iL, iR, -1);
  const midY = (r) => r[Math.floor(r.length / 2)][1];
  const [frontRun, backRun] = midY(fwd) > midY(back) ? [fwd, back] : [back, fwd];
  const crease = ringAt(P, run(inner, front), 0);
  const xL = B[iL][0], xR = B[iR][0], ye = B[iL][1];
  const kz = P(0, 0, 0)[1] - P(0, 0, 1)[1]; // screen units per unit of height
  const defs = [
    el("path", { id: `${id}bf`, d: pathOf(B, true), stroke: "none" }),
    el("path", { id: `${id}bs`, d: pathOf(frontRun), fill: "none" }),
    el("path", { id: `${id}sf`, d: `M${num(xL)} 0V-1H${num(xR)}V0z`, stroke: "none" }),
    el("path", { id: `${id}ss`, d: `M${num(xL)} 0V-1M${num(xR)} 0V-1`, fill: "none" }),
    el("path", { id: `${id}tf`, d: pathOf(B, true), stroke: "none" }),
    el("path", { id: `${id}ts`, d: pathOf(backRun), fill: "none" }),
    el("path", { id: `${id}tc`, d: pathOf(crease), fill: "none" }),
  ];
  const use = (part, attrs = {}) => el("use", { href: `#${id}${part}`, ...attrs });

  /**
   * One column at world (x, y): `heights` is a sampled track [[t, h], …] over
   * the loop (or a number for a still column), `stroke` a role or a track of
   * roles for its silhouette.
   */
  function column(x, y, heights, T, { stroke = "edge", crease = "mid", tol = 0.3, extra = [] } = {}) {
    const [ox, oy] = [P(x, y, 0)[0] - P(0, 0, 0)[0], P(x, y, 0)[1] - P(0, 0, 0)[1]];
    let keysH;
    if (typeof heights === "number") keysH = [[0, heights]];
    else {
      const idx = simplify(heights.map(([t, h]) => [t, [h * kz]]), tol);
      keysH = idx.map((i) => heights[i]);
    }
    const h0 = keysH[0][1];
    const lin = (f) => keysH.map(([t, h]) => [t, f(Math.max(0.6, h)), "linear"]);
    const scale = animate("transform", lin((h) => `1 ${num(h * kz)}`), T, { type: "scale" });
    const lift = animate("transform", lin((h) => `0 ${num(-h * kz)}`), T, { type: "translate" });
    const strokeAnim = Array.isArray(stroke) ? animateRole("stroke", stroke, T) : null;
    const s0 = Array.isArray(stroke) ? stroke[0][1] : stroke;
    return g({ transform: `translate(${num(ox)} ${num(oy)})`, stroke: role(s0) }, [
      strokeAnim,
      use("bf"), use("bs"),
      g({ transform: `translate(0 ${num(ye)})` }, [
        g({ transform: `scale(1 ${num(Math.max(0.6, h0) * kz)})` }, [scale, use("sf"), use("ss")]),
      ]),
      g({ transform: `translate(0 ${num(-Math.max(0.6, h0) * kz)})` }, [lift, use("tf"), use("ts"), use("tc", { stroke: role(crease) }), ...extra]),
    ]);
  }
  return { defs, column, kz };
}

/** Samples f(t) over [0, T] at `hz`. */
export const sample = (T, hz, f) => {
  const n = Math.round(T * hz), out = [];
  for (let i = 0; i <= n; i++) { const t = (i / n) * T; out.push([t, f(t)]); }
  return out;
};
