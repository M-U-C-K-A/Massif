/**
 * Blocks: six of the game's blocks on a slab, each told by what is drawn on
 * its faces: a grass block's turf line, a log's rings, bricks, ore, glass,
 * sand. One after another they are picked: lifted, with the bright stroke, and
 * set down as the next one rises.
 */
import { HL, writeFigure, el, g, role, num, pathOf, animate, animateRole } from "../lib.mjs";
import { stillPrism, rect } from "../solids.mjs";

const { Cam, fit, proj, facing, rings, prism, rrect, ringAt, poly } = HL;

const S = 20, GAP = 11, HT = 20, LIFT = 11;

export default function blocks(dir) {
  const W = 400, H = 320, T = 7.2;
  const X0 = -7, Y0 = -7, X1 = 3 * S + 2 * GAP + 7, Y1 = 2 * S + GAP + 7;
  const C = Cam(45, 0.5, 2.46);
  fit(C, [[X0, Y0, -4], [X1, Y1, -4], [X1, Y0, -4], [X0, Y1, -4], [0, 0, HT + LIFT], [X1, Y0, HT + LIFT]], W / 2, H / 2 + 4);
  const P = proj(C), front = facing(C);
  const kids = [];
  const [pr, pi] = rings(X0, Y0, X1, Y1, 7, 1.8);
  kids.push(stillPrism(P, front, pr, pi, -4, 0));

  // faces: +x at x = x1 and +y at y = y1 are the two the camera sees
  const fx = (x1, y, z) => P(x1, y, z), fy = (y1, x, z) => P(x, y1, z);
  const line = (pts, r = "mid") => el("path", { d: pathOf(pts), fill: "none", stroke: role(r) });
  const loop = (pts, r = "mid") => el("path", { d: pathOf(pts, true), fill: "none", stroke: role(r) });
  const dot = ([x, y], r = "edge", rad = 1.25) => el("ellipse", { cx: num(x), cy: num(y), rx: rad, ry: rad * 0.75, fill: role(r), stroke: "none" });

  const marks = {
    grass(x0, y0, x1, y1) {
      // the turf's ragged lower edge, on both faces
      const zs = [15, 14, 15.2, 13.2, 15, 14.4, 15.4];
      const along = (a, b, f) => zs.map((z, k) => f(a + ((b - a) * k) / (zs.length - 1), z));
      return [line(along(y0 + 3.5, y1 - 3.5, (y, z) => fx(x1, y, z))), line(along(x0 + 3.5, x1 - 3.5, (x, z) => fy(y1, x, z)))];
    },
    log(x0, y0, x1, y1) {
      const cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
      const ring = (k) => poly(ringAt(P, rrect(cx - k, cy - k, cx + k, cy + k, k * 0.8, 5), HT));
      return [
        el("path", { d: pathOf(ringAt(P, rrect(cx - 6, cy - 6, cx + 6, cy + 6, 4.8, 5), HT), true), fill: "none", stroke: role("mid") }),
        el("path", { d: pathOf(ringAt(P, rrect(cx - 2.6, cy - 2.6, cx + 2.6, cy + 2.6, 2, 5), HT), true), fill: "none", stroke: role("mid") }),
        ...[7, 13].map((d) => line([fx(x1, y0 + d, 2.5), fx(x1, y0 + d, HT - 2.5)], "lo")),
        ...[7, 13].map((d) => line([fy(y1, x0 + d, 2.5), fy(y1, x0 + d, HT - 2.5)], "lo")),
      ];
    },
    bricks(x0, y0, x1, y1) {
      const out = [];
      for (const z of [5, 10, 15]) {
        out.push(line([fx(x1, y0 + 3, z), fx(x1, y1 - 3, z)]), line([fy(y1, x0 + 3, z), fy(y1, x1 - 3, z)]));
      }
      const joints = [[0, [7, 15]], [5, [3.5, 11, 17]], [10, [7, 15]], [15, [3.5, 11, 17]]];
      for (const [z, ds] of joints) for (const d of ds) {
        if (d < 3 || d > S - 3) continue;
        out.push(line([fx(x1, y0 + d, z + 0.8), fx(x1, y0 + d, z + 4.2)]), line([fy(y1, x0 + d, z + 0.8), fy(y1, x0 + d, z + 4.2)]));
      }
      return out;
    },
    ore(x0, y0, x1, y1) {
      const f = [[5, 14], [8, 12], [13, 6], [15, 15], [6, 5]];
      return [...f.map(([d, z]) => dot(fx(x1, y0 + d, z))), ...f.map(([d, z]) => dot(fy(y1, x1 - d, z + 1))), dot(P(x0 + 7, y0 + 6, HT)), dot(P(x0 + 13, y0 + 12, HT))];
    },
    glass(x0, y0, x1, y1) {
      const frame = (f) => [f(3.2, 3.2), f(S - 3.2, 3.2), f(S - 3.2, HT - 3.2), f(3.2, HT - 3.2)];
      return [
        loop(frame((d, z) => fx(x1, y0 + d, z))),
        loop(frame((d, z) => fy(y1, x0 + d, z))),
        el("path", { d: pathOf(ringAt(P, rrect(x0 + 3.2, y0 + 3.2, x1 - 3.2, y1 - 3.2, 1.4, 3), HT), true), fill: "none", stroke: role("mid") }),
        line([fx(x1, y0 + 7, 13), fx(x1, y0 + 11, 8)], "lo"),
      ];
    },
    sand(x0, y0, x1, y1) {
      const f = [[4, 6], [9, 4], [15, 7], [6, 12], [12, 11], [16, 15], [5, 16], [10, 16]];
      return f.map(([a, b]) => dot(P(x0 + a, y0 + b, HT), "edge", 0.95));
    },
  };

  const layout = [
    ["log", 0, 0], ["bricks", 1, 0], ["glass", 2, 0],
    ["grass", 0, 1], ["ore", 1, 1], ["sand", 2, 1],
  ];
  const pick = ["grass", "ore", "sand", "glass", "bricks", "log"];
  const slot = T / pick.length;

  for (const [name, i, j] of layout.slice().sort((a, b) => a[1] + a[2] - (b[1] + b[2]))) {
    const x0 = i * (S + GAP), y0 = j * (S + GAP), x1 = x0 + S, y1 = y0 + S;
    const [ring, inner] = rings(x0, y0, x1, y1, 3.4, 1.2);
    const p = prism(P, front, ring, inner, 0, HT);
    const k = pick.indexOf(name), tUp = k * slot, tDown = tUp + slot;
    const dz = (P(0, 0, 0)[1] - P(0, 0, LIFT)[1]).toFixed(1);
    // the first one is up at rest: it comes down at the end of its slot and goes up again as the loop ends
    const lift = k === 0
      ? [[0, `0 -${dz}`, "lift"], [tDown, `0 -${dz}`, "lift"], [tDown + 0.7, "0 0", "linear"], [T - 0.7, "0 0", "lift"]]
      : [[0, "0 0", "linear"], [tUp, "0 0", "lift"], [tUp + 0.7, `0 -${dz}`, "linear"], [tDown, `0 -${dz}`, "lift"], [tDown + 0.7, "0 0", "linear"]];
    const strokeKeys = k === 0 ? [[0, "hi"], [tDown + 0.15, "edge"], [T - 0.55, "hi"]] : [[0, "edge"], [tUp + 0.15, "hi"], [tDown + 0.15, "edge"]];
    kids.push(g({ transform: k === 0 ? `translate(0 -${dz})` : undefined }, [
      animate("transform", lift, T, { type: "translate" }),
      el("path", { d: reDsil(p.sil), stroke: role(strokeKeys[0][1]) }, [animateRole("stroke", strokeKeys, T)]),
      el("path", { d: reDsil(p.crease), fill: "none", stroke: role("mid") }),
      ...marks[name](x0, y0, x1, y1),
    ]));
  }
  writeFigure(dir, "blocks", { w: W, h: H, view: [54, 51, 292, 224], title: "Six blocks of the game on a slab, picked one after another", body: [g({}, kids)] });
}

import { reD as reDsil } from "../lib.mjs";
