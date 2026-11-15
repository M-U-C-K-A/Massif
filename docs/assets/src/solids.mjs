/** Hairline solids whose top moves over time, as SVG elements with SMIL morphs. */
import { HL, el, g, reD, animate, animateRole, role } from "./lib.mjs";

const { prism } = HL;

/**
 * A rounded prism standing from z0, its top following `keys` ([t, z1, ease]).
 * The kernel's prism() gives the silhouette and the crease at every key; their
 * point counts never change with the height, so SMIL morphs one into the next.
 * `stroke` is the silhouette's role, or a track of roles ([t, role]).
 */
export function risingPrism(P, front, ring, inner, z0, keys, T, { stroke = "edge", crease = "mid", once = false, base = null } = {}) {
  const at = (z1) => prism(P, front, ring, inner, z0, Math.max(z0 + 0.6, z1));
  const z0v = base ?? keys[keys.length - 1][1];
  const rest = at(z0v);
  const silKeys = keys.map(([t, z, e]) => [t, reD(at(z).sil), e]);
  const crKeys = keys.map(([t, z, e]) => [t, reD(at(z).crease), e]);
  const sil = el("path", { d: reD(rest.sil), stroke: Array.isArray(stroke) ? role(stroke[0][1]) : role(stroke) }, [
    animate("d", silKeys, T, { once }),
    Array.isArray(stroke) ? animateRole("stroke", stroke, T, { once }) : null,
  ]);
  const crTrack = Array.isArray(crease);
  const cr = el("path", { d: reD(rest.crease), fill: "none", stroke: role(crTrack ? crease[0][1] : crease) }, [
    animate("d", crKeys, T, { once }),
    crTrack ? animateRole("stroke", crease, T, { once }) : null,
  ]);
  return g({}, [sil, cr]);
}

/** A still prism. */
export function stillPrism(P, front, ring, inner, z0, z1, { stroke = "edge", crease = "mid" } = {}) {
  const p = prism(P, front, ring, inner, z0, z1);
  return g({}, [
    el("path", { d: reD(p.sil), stroke: role(stroke) }),
    el("path", { d: reD(p.crease), fill: "none", stroke: role(crease) }),
  ]);
}

const { ringAt, run, hull, poly, open, rrect } = HL;

/**
 * A solid whose two rings differ (a foot smaller than its top, or larger), as
 * rules.md writes it: the silhouette is the hull of both rings, the crease the
 * front run of `inner` at z1. Returns kernel path strings.
 */
export function taper(P, front, foot, top, inner, z0, z1) {
  return {
    sil: poly(hull(ringAt(P, foot, z0).concat(ringAt(P, top, z1)))),
    crease: inner ? open(ringAt(P, run(inner, front), z1)) : "",
  };
}

/** A rounded rectangle footprint centred on (x, y), and its crease ring inset by b. */
export function rect(x, y, w, d, r, b, n = 6) {
  return [rrect(x - w / 2, y - d / 2, x + w / 2, y + d / 2, r, n), rrect(x - w / 2 + b, y - d / 2 + b, x + w / 2 - b, y + d / 2 - b, Math.max(0.5, r - b), n)];
}

/** Draws a kernel {sil, crease} pair as two paths. */
export function drawPair(p, { stroke = "edge", crease = "mid", silAttrs = {}, extra = [] } = {}) {
  return g({}, [
    el("path", { d: reD(p.sil), stroke: role(stroke), ...silAttrs }),
    p.crease ? el("path", { d: reD(p.crease), fill: "none", stroke: role(crease) }) : null,
    ...extra,
  ]);
}
