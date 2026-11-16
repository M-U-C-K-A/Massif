/**
 * Mods: a mod's jar on a slab beside four empty places. Its lid lifts, and
 * one after another its blocks come out and settle in their places, each told
 * by what is drawn on its faces: half-timbering, roof shingles, a tiled top,
 * cobbles. The one coming out has the bright stroke. Then they go back.
 */
import { HL, writeFigure, el, g, role, num, pathOf, animate, animateRole, animateShow, reD, boxed } from "../lib.mjs";
import { stillPrism } from "../solids.mjs";

const { Cam, fit, proj, facing, rings, prism, rrect } = HL;

const S = 18, GAP = 9, HT = 18, DROP = 26;

export default function mods(dir) {
  const W = 400, H = 320, T = 7.6;
  const JX = 0, JY = 0, JR = 11, JH = 30; // the jar, at the back left
  const BX = 2 * JR + 12; // the places start to its right
  const X0 = -8, Y0 = -8, X1 = BX + 2 * S + GAP + 8, Y1 = Math.max(2 * JR, 2 * S + GAP) + 8;
  const C = Cam(45, 0.5, 2.3);
  fit(C, [[X0, Y0, -4], [X1, Y1, -4], [X1, Y0, -4], [X0, Y1, -4], [0, 0, JH + 8], [X1, Y0, HT + DROP]], W / 2, H / 2 + 4);
  const P = boxed(proj(C)), front = facing(C);
  const kids = [];
  const [pr, pi] = rings(X0, Y0, X1, Y1, 7, 1.8);
  kids.push(stillPrism(P, front, pr, pi, -4, 0));

  const line = (pts, r = "mid") => el("path", { d: pathOf(pts), fill: "none", stroke: role(r) });
  const loop = (pts, r = "mid") => el("path", { d: pathOf(pts, true), fill: "none", stroke: role(r) });
  // the two faces the camera sees: +x at x1 and +y at y1
  const fx = (x1, y, z) => P(x1, y, z), fy = (y1, x, z) => P(x, y1, z);
  const marks = {
    timber(x0, y0, x1, y1) {
      const out = [];
      for (const [f, a, b] of [[(d, z) => fx(x1, y0 + d, z), 0, S], [(d, z) => fy(y1, x0 + d, z), 0, S]]) {
        out.push(line([f(a + 3, 3), f(a + 3, HT - 3)], "mid"), line([f(b - 3, 3), f(b - 3, HT - 3)], "mid"), line([f(a + 3, HT / 2), f(b - 3, HT / 2)], "mid"));
        out.push(line([f(a + 3, 3), f(b - 3, HT / 2)], "lo"), line([f(a + 3, HT - 3), f(b - 3, HT / 2)], "lo"));
      }
      return out;
    },
    shingles(x0, y0, x1, y1) {
      const out = [];
      for (const [f] of [[(d, z) => fx(x1, y0 + d, z)], [(d, z) => fy(y1, x0 + d, z)]]) {
        for (const z of [5, 9.5, 14]) {
          const row = [];
          for (let d = 2.5; d <= S - 2.5 + 1e-6; d += 0.5) row.push(f(d, z + 1.3 * Math.abs(Math.sin(((d - 2.5) / 3.25) * Math.PI))));
          out.push(line(row, "mid"));
        }
      }
      return out;
    },
    tiles(x0, y0, x1, y1) {
      const out = [];
      for (const k of [1, 2]) {
        const u = x0 + 2 + ((S - 4) * k) / 3, v = y0 + 2 + ((S - 4) * k) / 3;
        out.push(line([P(u, y0 + 2.5, HT), P(u, y1 - 2.5, HT)]), line([P(x0 + 2.5, v, HT), P(x1 - 2.5, v, HT)]));
      }
      out.push(line([fx(x1, y0 + 3, HT - 4), fx(x1, y1 - 3, HT - 4)], "lo"), line([fy(y1, x0 + 3, HT - 4), fy(y1, x1 - 3, HT - 4)], "lo"));
      return out;
    },
    cobble(x0, y0, x1, y1) {
      const stone = (f, d, z, w, h) => loop(rrect(d - w / 2, z - h / 2, d + w / 2, z + h / 2, Math.min(w, h) * 0.45, 3).map((q) => f(q.u, q.v)));
      const fxs = (d, z) => fx(x1, y0 + d, z), fys = (d, z) => fy(y1, x0 + d, z);
      return [[5, 5, 6, 4.5], [12.5, 4.5, 7, 4], [8.5, 10.5, 7.5, 4.6], [4.5, 15, 5, 3.6], [13.5, 14.5, 5.5, 4]].flatMap(([d, z, w, h]) => [stone(fxs, d, z, w, h), stone(fys, S - d, z, w, h)]);
    },
  };

  // the jar: a round body, its lid lifting while the blocks come out
  const body = rrect(JX, JY, JX + 2 * JR, JY + 2 * JR, JR, 8), bodyIn = rrect(JX + 1.4, JY + 1.4, JX + 2 * JR - 1.4, JY + 2 * JR - 1.4, JR - 1.4, 8);
  const lid = rrect(JX - 1, JY - 1, JX + 2 * JR + 1, JY + 2 * JR + 1, JR + 1, 8), lidIn = rrect(JX + 0.6, JY + 0.6, JX + 2 * JR - 0.6, JY + 2 * JR - 0.6, JR - 0.6, 8);
  const pb = prism(P, front, body, bodyIn, 0, JH), pl = prism(P, front, lid, lidIn, JH, JH + 5);
  for (const z of [0, JH + 5]) for (const q of lid) P(q.u, q.v, z);
  const tOpen = 0.4, tClose = 6.6, OPEN = 9;
  const dz = (z) => (P(0, 0, 0)[1] - P(0, 0, z)[1]).toFixed(1);
  kids.push(g({}, [el("path", { d: reD(pb.sil), stroke: role("edge") }), el("path", { d: reD(pb.crease), fill: "none", stroke: role("mid") }),
    // the jar's label band
    el("path", { d: pathOf(run2(body, front).map((q) => P(q.u, q.v, JH * 0.62))), fill: "none", stroke: role("lo") }),
    el("path", { d: pathOf(run2(body, front).map((q) => P(q.u, q.v, JH * 0.32))), fill: "none", stroke: role("lo") }),
  ]));
  kids.push(g({}, [
    animate("transform", [[0, "0 0", "linear"], [tOpen, "0 0", "lift"], [tOpen + 0.6, `0 -${dz(OPEN)}`, "linear"], [tClose, `0 -${dz(OPEN)}`, "lift"], [tClose + 0.6, "0 0", "linear"]], T, { type: "translate" }),
    el("path", { d: reD(pl.sil), stroke: role("edge") }), el("path", { d: reD(pl.crease), fill: "none", stroke: role("mid") }),
  ]));

  // the places, and the blocks coming to them from the jar's mouth
  const layout = [["timber", 1, 0], ["shingles", 0, 0], ["tiles", 1, 1], ["cobble", 0, 1]];
  const order = ["shingles", "timber", "cobble", "tiles"];
  const tFirst = 1.1, EACH = 1.05, MOVE = 0.8;
  for (const [name, i, j] of layout.slice().sort((a, b) => a[1] + a[2] - (b[1] + b[2]))) {
    const x0 = BX + i * (S + GAP), y0 = j * (S + GAP), x1 = x0 + S, y1 = y0 + S;
    // the empty place: a faint footprint
    kids.push(el("path", { d: pathOf(rrect(x0, y0, x1, y1, 3.4, 4).map((q) => P(q.u, q.v, 0)), true), fill: "none", stroke: role("lo"), "stroke-dasharray": "2 2" }));
    const [ring, inner] = rings(x0, y0, x1, y1, 3.4, 1.2);
    const p = prism(P, front, ring, inner, 0, HT);
    for (const q of ring) { P(q.u, q.v, 0); P(q.u, q.v, HT); }
    const k = order.indexOf(name), t0 = tFirst + k * EACH;
    // from above the jar's mouth to its place: one translation in the screen
    const from = P(JX + JR - S / 2, JY + JR - S / 2, JH + 6), to = P(x0, y0, 0);
    for (const [u, v] of [[0, 0], [S, 0], [0, S], [S, S]]) P(JX + JR - S / 2 + u, JY + JR - S / 2 + v, JH + 6 + HT);
    const off = `${num(from[0] - to[0])} ${num(from[1] - to[1])}`;
    const up = `${num(from[0] - to[0])} ${num(from[1] - to[1] - 14)}`;
    kids.push(g({ transform: `translate(${off})` }, [
      animate("transform", [[0, off, "linear"], [t0, off, "out"], [t0 + 0.35, up, "lift"], [t0 + 0.35 + MOVE, "0 0", "linear"], [tClose, "0 0", "in"], [tClose + 0.5, off, "linear"]], T, { type: "translate" }),
      animateShow([[0, false], [t0, true], [tClose + 0.5, false]], T),
      // the bright stroke goes with the block coming out, and stays on the last one while they rest
      el("path", { d: reD(p.sil), stroke: role("edge") }, [animateRole("stroke", [[0, "edge"], [t0, "aqua"], [k === order.length - 1 ? tClose : t0 + EACH, "edge"]], T)]),
      el("path", { d: reD(p.crease), fill: "none", stroke: role("mid") }),
      ...marks[name](x0, y0, x1, y1),
    ]));
  }
  writeFigure(dir, "mods", { w: W, h: H, view: P.view(6), title: "A mod's jar opens and its blocks come out, one after another, to their places beside the game's", body: [g({}, kids)] });
}

/** The front half of a ring, as the camera sees it. */
function run2(ringPts, front) {
  return HL.run(ringPts, front);
}
