/**
 * A small library for drawing Hairline figures as animated SVG files.
 *
 * The geometry comes from the Hairline kernel itself (`kernel.js`, from the
 * hairline-create skill; HAIRLINE_KERNEL names another copy), loaded in a
 * sandbox without a DOM. On top of it: an element tree, a renderer that
 * resolves colour roles per theme, and SMIL animations built from keyframes,
 * so the files play in an <img> on GitHub without a line of script.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";

export function loadHL(file = process.env.HAIRLINE_KERNEL || path.join(os.homedir(), ".claude/skills/hairline-create/kernel.js")) {
  const ctx = { Math, performance: { now: () => 0 }, matchMedia: () => ({ matches: false, addEventListener() {} }) };
  ctx.window = ctx;
  ctx.globalThis = ctx;
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(file, "utf8") + "\n;globalThis.HL = HL;", ctx);
  return ctx.HL;
}

export const HL = loadHL();

/* The palette: Hairline's four greys, its ground the page's own colour on GitHub, and Massif's accent for the one bright stroke. */
export const THEMES = {
  light: { plate: "#ffffff", hi: "#f0601a", aqua: "#0a8fb0", ink: "#232327", edge: "#9b9ba4", mid: "#c3c3c9", lo: "#e2e2e6", none: "none" },
  dark: { plate: "#0d1117", hi: "#ff7028", aqua: "#3ccbe6", ink: "#d0d6e0", edge: "#6a6d75", mid: "#43464d", lo: "#2b2e34", none: "none" },
};
/* The figures of the Aquatic Update (1.0) draw their one bright stroke in `aqua`, the colour of its banner, rather than `hi`. */

/* Easing curves, as SMIL key splines. */
export const EASE = {
  lift: "0.32 0.72 0 1", // Hairline's 700 ms tween
  inout: "0.65 0 0.35 1",
  out: "0.22 1 0.36 1",
  in: "0.55 0 1 0.45",
  linear: "0 0 1 1",
};

/* ---- numbers and paths ---- */

const tenths = (v) => Math.round(v * 10);
const fmtT = (t) => {
  // an integer number of tenths, as the shortest decimal
  const neg = t < 0, a = Math.abs(t), i = Math.floor(a / 10), d = a % 10;
  let s = d ? (i ? `${i}.${d}` : `.${d}`) : `${i}`;
  return neg && a ? "-" + s : s;
};
export const num = (v) => fmtT(tenths(v));
/* Joins numbers with the fewest separators SVG allows: none before a minus sign, none before a point when the last number has one. */
const join = (nums) => nums.reduce((s, n, i) => {
  if (i === 0 || n.startsWith("-")) return s + n;
  const prev = nums[i - 1];
  return n.startsWith(".") && prev.includes(".") ? s + n : s + " " + n;
}, "");

/** A path through screen points, in relative commands rounded to a tenth: the same point count always gives the same commands, so SMIL can morph it. */
export function pathOf(points, closed = false) {
  if (!points.length) return "";
  const q = points.map(([x, y]) => [tenths(x), tenths(y)]);
  let s = "M" + join([fmtT(q[0][0]), fmtT(q[0][1])]);
  if (q.length > 1) {
    const rest = [];
    for (let i = 1; i < q.length; i++) rest.push(fmtT(q[i][0] - q[i - 1][0]), fmtT(q[i][1] - q[i - 1][1]));
    s += "l" + join(rest);
  }
  return closed ? s + "z" : s;
}
/** Several open runs as one path. */
export const pathsOf = (runs) => runs.map((r) => pathOf(r)).join("");

/** Converts an absolute path string from the kernel (M x y L x y … Z) into runs of points. */
export function runsOf(d) {
  const runs = [];
  const re = /([MLZ])([^MLZ]*)/g;
  let m, cur = null;
  while ((m = re.exec(d))) {
    const nums = m[2].trim().split(/[\s,]+/).filter(Boolean).map(Number);
    if (m[1] === "M") { cur = { pts: [[nums[0], nums[1]]], closed: false }; runs.push(cur); }
    else if (m[1] === "L") { for (let i = 0; i < nums.length; i += 2) cur.pts.push([nums[i], nums[i + 1]]); }
    else if (m[1] === "Z") cur.closed = true;
  }
  return runs;
}
/** Re-encodes a kernel path string in relative tenths. */
export const reD = (d) => runsOf(d).map((r) => pathOf(r.pts, r.closed)).join("");

/* ---- the element tree ---- */

export const el = (tag, attrs = {}, kids = []) => ({ tag, attrs, kids });
export const g = (attrs, kids) => el("g", attrs, kids);

/** A colour role, resolved per theme. */
export const role = (name) => ({ role: name });

function attrValue(v, theme) {
  if (v && typeof v === "object" && "role" in v) return THEMES[theme][v.role];
  if (Array.isArray(v)) return v.map((x) => attrValue(x, theme)).join(";");
  return typeof v === "number" ? num(v) : String(v);
}

const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");

export function renderNode(n, theme, depth = 0) {
  if (typeof n === "string") return esc(n);
  const a = Object.entries(n.attrs)
    .filter(([, v]) => v !== undefined && v !== null && v !== false)
    .map(([k, v]) => ` ${k}="${esc(attrValue(v, theme))}"`).join("");
  const kids = n.kids.filter(Boolean);
  if (!kids.length) return `<${n.tag}${a}/>`;
  return `<${n.tag}${a}>${kids.map((k) => renderNode(k, theme, depth + 1)).join("")}</${n.tag}>`;
}

/* ---- timelines ---- */

/**
 * A keyframe track: [[t, v, ease], …] with t in seconds over the loop. `ease`
 * names the curve from this key to the next. The track is closed into the
 * loop (the value at T is the value at 0) unless `once` is set.
 */
export function animate(attr, keys, T, { fmt = (v) => v, discrete = false, once = false, type = null } = {}) {
  let k = keys.slice().sort((a, b) => a[0] - b[0]);
  if (!k.length) return null;
  if (k[0][0] > 0) k.unshift([0, once ? k[0][1] : k[k.length - 1][1], once ? "linear" : k[k.length - 1][2] || "linear"]);
  if (k[k.length - 1][0] < T) k.push([T, once ? k[k.length - 1][1] : k[0][1], "linear"]);
  // drop keys that change nothing between equal neighbours
  const vals = k.map((x) => fmt(x[1]));
  const keep = [];
  for (let i = 0; i < k.length; i++) {
    if (i > 0 && i < k.length - 1 && vals[i] === vals[i - 1] && vals[i] === vals[i + 1]) continue;
    keep.push(i);
  }
  k = keep.map((i) => k[i]);
  const values = keep.map((i) => vals[i]);
  if (values.every((v) => v === values[0])) return null; // nothing moves
  const keyTimes = k.map((x) => +(x[0] / T).toFixed(4));
  keyTimes[0] = 0;
  keyTimes[keyTimes.length - 1] = 1;
  const attrs = {
    attributeName: type ? "transform" : attr,
    type: type || undefined,
    dur: `${+T.toFixed(3)}s`,
    repeatCount: once ? undefined : "indefinite",
    fill: once ? "freeze" : undefined,
    values: values.join(";"),
    keyTimes: keyTimes.join(";"),
  };
  if (discrete) attrs.calcMode = "discrete";
  else {
    attrs.calcMode = "spline";
    attrs.keySplines = k.slice(0, -1).map((x) => EASE[x[2] || "linear"] || x[2]).join(";");
  }
  return el(type ? "animateTransform" : "animate", attrs);
}

/** A colour role track, switched without easing. */
export function animateRole(attr, keys, T, opts = {}) {
  const fmt = (v) => v; // roles are resolved at render time; compare by name
  const a = animate(attr, keys.map(([t, r]) => [t, r, "linear"]), T, { ...opts, discrete: true, fmt });
  if (!a) return null;
  const names = a.attrs.values.split(";");
  a.attrs.values = names.map((r) => role(r));
  return a;
}

/** Ramer–Douglas–Peucker over a sampled track of vectors, keeping the keys a linear interpolation needs within `tol`. */
export function simplify(samples, tol) {
  // samples: [[t, [v0, v1, …]], …]
  const n = samples.length;
  if (n <= 2) return samples.map((_, i) => i);
  const keep = new Uint8Array(n);
  keep[0] = keep[n - 1] = 1;
  const stack = [[0, n - 1]];
  while (stack.length) {
    const [a, b] = stack.pop();
    let worst = -1, wi = -1;
    const [ta, va] = samples[a], [tb, vb] = samples[b];
    for (let i = a + 1; i < b; i++) {
      const [ti, vi] = samples[i], u = (ti - ta) / (tb - ta);
      let e = 0;
      for (let j = 0; j < vi.length; j++) e = Math.max(e, Math.abs(vi[j] - (va[j] + (vb[j] - va[j]) * u)));
      if (e > worst) { worst = e; wi = i; }
    }
    if (worst > tol) { keep[wi] = 1; stack.push([a, wi], [wi, b]); }
  }
  const out = [];
  for (let i = 0; i < n; i++) if (keep[i]) out.push(i);
  return out;
}

/**
 * A projection that remembers the box of every point it projects, so a
 * figure can frame itself: `view(margin, aspect)` gives the viewBox of all it
 * drew, widened or heightened to `aspect` around its middle.
 */
export function boxed(P0) {
  let a = 1e9, b = -1e9, c = 1e9, d = -1e9;
  const P = (x, y, z) => { const q = P0(x, y, z); a = Math.min(a, q[0]); b = Math.max(b, q[0]); c = Math.min(c, q[1]); d = Math.max(d, q[1]); return q; };
  P.view = (margin = 6, aspect = 1.48) => {
    let w = b - a + 2 * margin, h = d - c + 2 * margin;
    const cx = (a + b) / 2, cy = (c + d) / 2;
    if (w / h < aspect) w = h * aspect; else h = w / aspect;
    return [cx - w / 2, cy - h / 2, w, h].map((v) => Math.round(v));
  };
  return P;
}

/* ---- the document ---- */

/**
 * Writes `<name>-light.svg` and `<name>-dark.svg`, framed on `view` (the
 * drawing's box over its whole loop, with a margin: `sheet.mjs` measures it).
 * The background stays
 * transparent: plates are filled with the page's own colour, as Hairline fills
 * them with its ground, so the far edges they hide stay hidden.
 */
export function writeFigure(dir, name, { w, h, view = [0, 0, w, h], title, body }) {
  for (const theme of ["light", "dark"]) {
    const root = el("svg", {
      xmlns: "http://www.w3.org/2000/svg",
      viewBox: view.join(" "),
      width: view[2],
      height: view[3],
      fill: role("plate"),
      stroke: role("mid"),
      "stroke-width": "0.9",
      "stroke-linejoin": "round",
      "stroke-linecap": "round",
      role: "img",
      "aria-label": title,
    }, [
      el("title", {}, [title]),
      el("style", {}, ["*{vector-effect:non-scaling-stroke}"]),
      ...body,
    ]);
    const svg = renderNode(root, theme) + "\n";
    fs.writeFileSync(path.join(dir, `${name}-${theme}.svg`), svg);
  }
  const size = fs.statSync(path.join(dir, `${name}-light.svg`)).size;
  console.log(`${name}: ${(size / 1024).toFixed(1)} KB per theme`);
}

/** Shows and hides an element ([t, shown]): Chrome will not animate a paint to `none`, so visibility it is. */
export function animateShow(keys, T, opts = {}) {
  const a = animate("visibility", keys.map(([t, on]) => [t, on ? "visible" : "hidden", "linear"]), T, { ...opts, discrete: true });
  return a;
}
