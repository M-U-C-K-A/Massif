/**
 * The mark: Massif's voxel peak, built as Hairline solids on a slab of ground.
 * Two pillars stand for the legs of the M, a lower block between them for its
 * saddle, two steps in front. They rise out of the slab once, from the middle
 * outwards, and the M takes the bright stroke when it has landed.
 */
import { HL, writeFigure, g } from "../lib.mjs";
import { risingPrism, stillPrism } from "../solids.mjs";

const { Cam, fit, proj, facing, rings } = HL;

export default function mark(dir) {
  const W = 260, H = 220;
  const c = 20, half = 8.8, r = 3.4, b = 1.4;
  const blocks = [
    // [x, y, height, delay, part of the M]
    [-0.785 * c, 0.785 * c, 2.6 * c, 0.34, true],
    [0.785 * c, -0.785 * c, 2.6 * c, 0.34, true],
    [0.57 * c, 0.57 * c, 1.72 * c, 0.1, true, 10.2],
    [0.215 * c, 1.785 * c, 0.8 * c, 0.22, false],
    [1.785 * c, 0.215 * c, 0.8 * c, 0.22, false],
  ];
  const lo = -1.5 * c, hi = 2.5 * c, PB = 7;
  const C = Cam(45, 0.5, 1.62);
  fit(C, [[lo, lo, -PB], [hi, hi, -PB], [hi, lo, -PB], [lo, hi, -PB], [blocks[0][0], blocks[0][1], blocks[0][2]], [blocks[1][0], blocks[1][1], blocks[1][2]]], W / 2, H / 2 + 2);
  const P = proj(C), front = facing(C);

  const T = 2.2, rise = 1.0;
  const kids = [];
  const [pr, pi] = rings(lo, lo, hi, hi, 9, 2.2);
  kids.push(stillPrism(P, front, pr, pi, -PB, 0, { stroke: "edge" }));
  // back to front: by x + y
  for (const [x, y, h, d, m, hw = half] of blocks.slice().sort((p, q) => p[0] + p[1] - (q[0] + q[1]))) {
    const [ring, inner] = rings(x - hw, y - hw, x + hw, y + hw, r, b);
    const keys = [[0, 0.6, "linear"], [d, 0.6, "lift"], [d + rise, h, "linear"]];
    const stroke = m ? [[0, "edge"], [1.5, "hi"]] : "edge";
    kids.push(risingPrism(P, front, ring, inner, 0, keys, T, { stroke, once: true, base: h }));
  }
  writeFigure(dir, "mark", { w: W, h: H, view: [32, 21, 196, 170], title: "Massif", body: [g({}, kids)] });
}
