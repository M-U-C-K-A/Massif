/**
 * Graph: a terrain graph as nodes on a board. Two sources (a mountain, a
 * noise) feed a combine, an erosion, and an output that holds the terrain it
 * makes. A change sends pulses down the wires, node after node; each node takes
 * the bright stroke while it computes, and the terrain on the output sinks,
 * then rises again when the result arrives.
 */
import { HL, writeFigure, el, g, role, reD, pathOf, num, animate, animateRole } from "../lib.mjs";
import { stillPrism, risingPrism, taper, rect, drawPair } from "../solids.mjs";

const { Cam, fit, proj, facing, rings, prism, rrect, open, seg } = HL;

export default function graph(dir) {
  const W = 400, H = 320, T = 7.2, TH = 5;
  const nodes = {
    mountain: { x: 0, y: -27, w: 30, d: 38 },
    noise: { x: 0, y: 27, w: 30, d: 38 },
    combine: { x: 56, y: 0, w: 30, d: 38 },
    erosion: { x: 108, y: 0, w: 30, d: 38 },
    output: { x: 166, y: 0, w: 42, d: 48 },
  };
  const C = Cam(45, 0.5, 1.56);
  const B = { x0: -24, y0: -55, x1: 196, y1: 55 };
  fit(C, [[B.x0, B.y0, -3], [B.x1, B.y1, -3], [B.x1, B.y0, -3], [B.x0, B.y1, -3], [166, 0, 30], [0, -27, 20]], W / 2, H / 2 + 2);
  const P = proj(C), front = facing(C);
  const kids = [];

  // the board
  const [br, bi] = rings(B.x0, B.y0, B.x1, B.y1, 10, 2.2);
  kids.push(stillPrism(P, front, br, bi, -3, 0));

  // timeline: the stale result sinks, pulses run node to node, the new one rises
  const tStart = 1.2, legs = [
    { from: ["mountain", "noise"], to: "combine", t0: tStart, t1: tStart + 0.8 },
    { from: ["combine"], to: "erosion", t0: tStart + 1.35, t1: tStart + 2.0 },
    { from: ["erosion"], to: "output", t0: tStart + 2.55, t1: tStart + 3.2 },
  ];
  const tArrive = legs[2].t1, busy = 0.55;
  const busyAt = { combine: legs[0].t1, erosion: legs[1].t1, output: tArrive };

  const port = (n, side, k = 0, of = 1) => {
    const c = nodes[n];
    const y = c.y + (of > 1 ? (k - (of - 1) / 2) * 14 : 0);
    return [side === "in" ? c.x - c.w / 2 + 5 : c.x + c.w / 2 - 5, y];
  };

  // the cards, back to front, each with what tells it apart
  const order = Object.keys(nodes).sort((a, b) => nodes[a].x + nodes[a].y - (nodes[b].x + nodes[b].y));
  for (const n of order) {
    const c = nodes[n], z1 = TH;
    const [ring, inner] = rect(c.x, c.y, c.w, c.d, 4, 1.1);
    const p = prism(P, front, ring, inner, 0, z1);
    let stroke = "edge";
    if (busyAt[n] !== undefined) stroke = [[0, n === "output" ? "hi" : "edge"], [tStart, "edge"], [busyAt[n], "hi"], ...(n === "output" ? [] : [[busyAt[n] + busy, "edge"]])];
    const parts = [
      el("path", { d: reD(p.sil), stroke: Array.isArray(stroke) ? role(stroke[0][1]) : role(stroke) }, [Array.isArray(stroke) ? animateRole("stroke", stroke, T) : null]),
      el("path", { d: reD(p.crease), fill: "none", stroke: role("mid") }),
      // a header strip, as on a node
      el("path", { d: reD(seg(P(c.x - c.w / 2 + 8, c.y - c.d / 2 + 3, z1), P(c.x - c.w / 2 + 8, c.y + c.d / 2 - 3, z1))), fill: "none", stroke: role("lo") }),
    ];
    const at = (x, y) => P(x, y, z1);
    if (n === "mountain") {
      const [foot] = rect(c.x + 4, c.y, 16, 16, 3, 0.5), [tip] = rect(c.x + 5, c.y + 1, 2.4, 2.4, 1, 0.3);
      parts.push(drawPair(taper(P, front, foot, tip, null, z1, z1 + 15)));
    } else if (n === "noise") {
      const w = [2, 0, 1, 1, 2, 0, 0, 1, 2], fills = ["edge", "mid", "lo"];
      w.forEach((k, i) => {
        const [sx, sy] = at(c.x + 4 + ((i % 3) - 1) * 6.5, c.y + (Math.floor(i / 3) - 1) * 8);
        parts.push(el("ellipse", { cx: num(sx), cy: num(sy), rx: 1.9, ry: 1.1, fill: role(fills[k]), stroke: "none" }));
      });
    } else if (n === "combine") {
      const [a, b] = [port(n, "in", 0, 2), port(n, "in", 1, 2)], o = port(n, "out");
      const mid = [c.x + 4, c.y];
      parts.push(el("path", { d: pathOf([at(...a), at(mid[0] - 2, a[1] * 0.4), at(...mid)]) + pathOf([at(...b), at(mid[0] - 2, b[1] * 0.4), at(...mid)]) + pathOf([at(...mid), at(...o)]), fill: "none", stroke: role("mid") }));
    } else if (n === "erosion") {
      for (const dy of [-9, 0, 9]) {
        const pts = [];
        for (let k = 0; k <= 12; k++) { const x = c.x - 8 + k * 1.6; pts.push(at(x, c.y + dy + 2.2 * Math.sin(k * 0.9 + dy))); }
        parts.push(el("path", { d: pathOf(pts), fill: "none", stroke: role("mid") }));
      }
    }
    kids.push(g({}, parts));

    // the output's terrain: 3 × 3 columns that sink when the graph changes and rise with the result
    if (n === "output") {
      const hs = [[6, 11, 7], [10, 18, 12], [5, 9, 6]], s = 11, f = 8.6;
      const cols = [];
      for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) cols.push({ i, j, h: hs[i][j] });
      cols.sort((a, b) => a.i + a.j - (b.i + b.j));
      for (const col of cols) {
        const x = c.x + 2 + (col.i - 1) * s, y = c.y + (col.j - 1) * s;
        const [r, ri] = rect(x, y, f, f, 2, 0.8);
        const d = Math.hypot(col.i - 1, col.j - 1) * 0.06;
        const keys = [[0, z1 + col.h, "linear"], [tStart + d, z1 + col.h, "lift"], [tStart + d + 0.7, z1 + 1.2, "linear"], [tArrive + d, z1 + 1.2, "lift"], [tArrive + d + 0.75, z1 + col.h, "linear"]];
        kids.push(risingPrism(P, front, r, ri, z1, keys, T, { base: z1 + col.h }));
      }
    }
  }

  // ports: dim dots on the cards
  for (const [n, side, k, of] of [["mountain", "out"], ["noise", "out"], ["combine", "in", 0, 2], ["combine", "in", 1, 2], ["combine", "out"], ["erosion", "in"], ["erosion", "out"], ["output", "in"]]) {
    const [sx, sy] = P(...port(n, side, k, of), TH);
    kids.push(el("ellipse", { cx: num(sx), cy: num(sy), rx: 2.2, ry: 1.3, fill: role("edge"), stroke: "none" }));
  }

  // wires, over the board between the cards, and the pulses that run along them
  const wire = (a, b) => {
    const pts = [];
    for (let k = 0; k <= 28; k++) {
      const u = k / 28, v = 1 - u;
      const x = v ** 3 * a[0] + 3 * v * v * u * (a[0] + 20) + 3 * v * u * u * (b[0] - 20) + u ** 3 * b[0];
      const y = v ** 3 * a[1] + 3 * v * v * u * a[1] + 3 * v * u * u * b[1] + u ** 3 * b[1];
      pts.push(P(x, y, TH));
    }
    return pts;
  };
  const wires = [
    [port("mountain", "out"), port("combine", "in", 0, 2), 0],
    [port("noise", "out"), port("combine", "in", 1, 2), 0],
    [port("combine", "out"), port("erosion", "in"), 1],
    [port("erosion", "out"), port("output", "in"), 2],
  ];
  for (const [a, b, leg] of wires) {
    const pts = wire(a, b), d = pathOf(pts);
    kids.push(el("path", { d, fill: "none", stroke: role("mid") }));
    const { t0, t1 } = legs[leg];
    const motion = el("animateMotion", {
      path: d, dur: `${T}s`, repeatCount: "indefinite", calcMode: "spline",
      keyPoints: "0;0;1;1", keyTimes: [0, t0 / T, t1 / T, 1].map((x) => +x.toFixed(4)).join(";"),
      keySplines: "0 0 1 1;0.65 0 0.35 1;0 0 1 1",
    });
    const show = animate("r", [[0, 0, "linear"], [t0, 2.1, "linear"], [t1 + 0.05, 0, "linear"]], T, { discrete: true });
    kids.push(el("circle", { r: 0, fill: role("hi"), stroke: "none" }, [motion, show]));
  }

  writeFigure(dir, "graph", { w: W, h: H, view: [12, 59, 376, 205], title: "A terrain graph: pulses run from two sources through a combine and an erosion to the output, whose terrain rises", body: [g({}, kids)] });
}
