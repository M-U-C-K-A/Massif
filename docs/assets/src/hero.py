"""The banner at the top of the README, in two themes.

    python3 docs/assets/src/hero.py            # docs/assets/hero-{dark,light}.svg
    python3 docs/assets/src/hero.py --at 1.2   # frozen 1.2 s in, to look at it

A card of the home screen's five night scenes, one after another, dithered as
Massif's home screen does it: at night for GitHub's dark theme, its own
sixteen colours a scene; at dawn for the light theme, the scene's relief
(its `_relief` layer) lifted into daylight under a sky drawn for it. Four
phases of the dither follow one another, so that the picture moves; stars
twinkle in the night skies only. Over it, Massif's mark of blocks rises and
its name falls into place in blocks, a letter after another.

Everything moves with CSS animations, which play inside an <img>; the
pictures are PNGs inside the SVG. Needs Pillow.
"""

import base64
import io
import math
import os
import random
import sys

from PIL import Image, ImageEnhance

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
BACKDROPS = os.path.join(HERE, "backdrops")
# Each scene's land cut out (its `_relief` layer), for the dawn of the light
# theme: kept here, as only the banner needs them.
LAYERS = os.path.join(HERE, "scenes")
OUT = os.path.join(ROOT, "docs", "assets")

W, H = 1280, 420
CELL = 3  # pixels of the card per cell of the dither
COLS, ROWS = W // CELL, H // CELL
RADIUS = 22
PHASES = 4
SLOT, FADE = 5.0, 1.2  # seconds a scene shows, and its cross-fade

# The scenes: their file, where the card's band is taken down the picture
# (0 top, 1 bottom), the dawn's horizon, and how much the land is lifted
# into daylight and saturated.
SCENES = [
    ("02-alpes", 0.42, (232, 226, 248), 2.5, 1.25),
    ("03-iles-volcaniques", 0.62, (255, 224, 206), 1.9, 0.72),
    ("01-canyon", 0.60, (255, 214, 176), 2.6, 1.3),
    ("04-avatar", 0.60, (214, 236, 242), 2.6, 1.3),
    ("05-dunes", 0.74, (240, 228, 246), 2.6, 1.25),
]
# The home screen's other scenes, for their dawn (the light theme's
# picture): not in the banner.
MORE = [
    ("06-fjord-aurore", 0.5, (212, 238, 236), 2.3, 1.15),
    ("07-rizieres", 0.6, (255, 214, 228), 2.5, 1.15),
    ("08-falaises-craie", 0.5, (226, 232, 248), 1.9, 1.05),
    ("09-foret-lac", 0.5, (255, 226, 202), 2.6, 1.15),
    ("10-exoplanete", 0.5, (236, 226, 250), 2.4, 1.15),
]

# Massif's three tones: the tops, the faces turned to the light, the others.
PEACH, ORANGE, RUST = "#fed7aa", "#f97316", "#9a3412"
PAPER = (252, 250, 246)
NIGHT = (6, 9, 18)

BAYER = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
]


def smooth(t):
    t = max(0.0, min(1.0, t))
    return t * t * (3 - 2 * t)


def band(path, at):
    """The card's band of the picture at `path`, a pixel a cell."""
    im = Image.open(path)
    w, h = im.size
    crop_h = int(w * H / W)
    top = int((h - crop_h) * at)
    return im.crop((0, top, w, top + crop_h)).resize((COLS, ROWS), Image.LANCZOS)


def palette_of(src, n):
    q = src.quantize(colors=n, method=Image.Quantize.MEDIANCUT)
    p = q.getpalette()[: 3 * n]
    return [tuple(p[i : i + 3]) for i in range(0, len(p), 3)]


def dither(src, palette, phase, veil):
    """`src` (RGB, a pixel a cell) in `palette`, ordered dither shifted by
    `phase`; `veil(x)` (amount, colour) quiets the picture where the words
    are."""
    out = Image.new("P", src.size)
    flat = [v for c in palette for v in c]
    out.putpalette(flat + [0] * (768 - len(flat)))
    px, o = src.load(), out.load()
    dx, dy = [(0, 0), (4, 4), (4, 0), (0, 4)][phase % 4]
    for y in range(src.size[1]):
        for x in range(src.size[0]):
            r, g, b = px[x, y]
            k, (vr, vg, vb) = veil(x)
            r, g, b = r + (vr - r) * k, g + (vg - g) * k, b + (vb - b) * k
            t = ((BAYER[(y + dy) % 8][(x + dx) % 8] + 0.5) / 64.0 - 0.5) * 34.0
            r, g, b = r + t, g + t, b + t
            best, bd = 0, 1e18
            for i, (pr, pg, pb) in enumerate(palette):
                d = (r - pr) ** 2 * 0.3 + (g - pg) ** 2 * 0.59 + (b - pb) ** 2 * 0.11
                if d < bd:
                    best, bd = i, d
            o[x, y] = best
    return out


def sky(top, horizon, size, at=0.7):
    """A dawn sky: `top` overhead to `horizon` low down."""
    w, h = size
    g = Image.new("RGB", size)
    px = g.load()
    for y in range(h):
        t = smooth((y / h) / at)
        c = tuple(int(top[i] + (horizon[i] - top[i]) * t) for i in range(3))
        for x in range(w):
            px[x, y] = c
    return g


def scene(name, at, horizon, lift, sat, theme):
    """The phases of the dither of a scene, and where its sky is (for
    stars): a mask, a pixel a cell, 255 where there is no land."""
    relief = band(os.path.join(LAYERS, f"{name}_relief.png"), at)
    land = relief.split()[3]
    if theme == "dark":
        src = band(os.path.join(BACKDROPS, f"{name}_complet.png"), at).convert("RGB")
        src = ImageEnhance.Contrast(ImageEnhance.Brightness(src).enhance(1.08)).enhance(1.16)
        target = NIGHT

        def veil(x):
            return 0.8 * (1 - smooth((x / COLS - 0.05) / 0.6)), target

    else:
        rgb = relief.convert("RGB")
        lifted = lambda im: ImageEnhance.Color(ImageEnhance.Contrast(ImageEnhance.Brightness(im).enhance(lift)).enhance(0.9)).enhance(sat)
        dawn = sky(PAPER, horizon, rgb.size)
        # What shines in the night sky — a far peak in the snow, a glow,
        # lava — stays, lifted; the dark of the night becomes the dawn.
        whole = band(os.path.join(BACKDROPS, f"{name}_complet.png"), at).convert("RGB")
        shine = whole.convert("L").point(lambda v: int(255 * smooth((v / 255.0 - 0.22) / 0.3)))
        # (Bright already: lifted a little, its colour kept.)
        glow = ImageEnhance.Color(ImageEnhance.Brightness(whole).enhance(1.3)).enhance(1.1)
        up = Image.composite(glow, dawn, shine)
        src = Image.composite(lifted(rgb), up, land)
        target = PAPER

        def veil(x):
            return 0.84 * (1 - smooth((x / COLS - 0.05) / 0.6)), target

    pal = palette_of(src, 16)
    sky_mask = land.point(lambda a: 255 if a < 8 else 0)
    return [dither(src, pal, p, veil) for p in range(PHASES)], sky_mask


def dawn(name, horizon, lift, sat, size=(1920, 1200)):
    """The scene at dawn, whole, for the home screen in the light theme:
    its land lifted into daylight, under a sky drawn for it, what shines in
    the night sky (a moon, the northern lights, a glow) kept."""
    relief = Image.open(os.path.join(LAYERS, f"{name}_relief.png")).convert("RGBA").resize(size, Image.LANCZOS)
    whole = Image.open(os.path.join(BACKDROPS, f"{name}_complet.png")).convert("RGB").resize(size, Image.LANCZOS)
    land = relief.split()[3]
    day = ImageEnhance.Color(ImageEnhance.Contrast(ImageEnhance.Brightness(relief.convert("RGB")).enhance(lift)).enhance(0.9)).enhance(sat)
    # The sky: a column of the gradient, stretched across.
    column = sky(PAPER, horizon, (1, size[1]))
    up = column.resize(size, Image.NEAREST)
    shine = whole.convert("L").point(lambda v: int(255 * smooth((v / 255.0 - 0.22) / 0.3)))
    glow = ImageEnhance.Color(ImageEnhance.Brightness(whole).enhance(1.3)).enhance(1.1)
    return Image.composite(day, Image.composite(glow, up, shine), land)


def png_uri(im):
    buf = io.BytesIO()
    im.save(buf, "PNG", optimize=True)
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode("ascii")


# ------------------------------------------------------------- the words --

FONT = {
    "M": ["1...1", "11.11", "1.1.1", "1.1.1", "1...1", "1...1", "1...1"],
    "A": [".111.", "1...1", "1...1", "11111", "1...1", "1...1", "1...1"],
    "S": [".1111", "1....", "1....", ".111.", "....1", "....1", "1111."],
    "I": ["111", ".1.", ".1.", ".1.", ".1.", ".1.", "111"],
    "F": ["11111", "1....", "1....", "1111.", "1....", "1....", "1...."],
}


def collinear(a, b, c):
    return (b[0] - a[0]) * (c[1] - b[1]) == (b[1] - a[1]) * (c[0] - b[0])


def outline(cells, s, ox, oy):
    """The outline of grid cells (col, row), cells `s` wide from `(ox, oy)`,
    as an SVG path of closed loops."""
    # Edges with no cell beyond, clockwise; where cells touch at a corner,
    # the corner starts two.
    edges = {}
    for (c, r) in cells:
        for (a, b, n) in [((c, r), (c + 1, r), (c, r - 1)), ((c + 1, r), (c + 1, r + 1), (c + 1, r)), ((c + 1, r + 1), (c, r + 1), (c, r + 1)), ((c, r + 1), (c, r), (c - 1, r))]:
            if n not in cells:
                edges.setdefault(a, []).append(b)
    out = []
    while edges:
        start = next(iter(edges))
        loop, p = [start], edges[start].pop()
        if not edges[start]:
            del edges[start]
        while p != start:
            loop.append(p)
            nxt = edges[p].pop()
            if not edges[p]:
                del edges[p]
            p = nxt
        pts = [loop[i] for i in range(len(loop)) if not collinear(loop[i - 1], loop[i], loop[(i + 1) % len(loop)])]
        out.append("M" + " L".join(f"{ox + x * s:g},{oy + y * s:g}" for x, y in pts) + "Z")
    return "".join(out)


def letter(ch, x0, y0, s, d):
    """Letter `ch` in blocks at `(x0, y0)`: the tops and right sides of its
    blocks, `d` deep toward the upper right, and its front."""
    rows = FONT[ch]
    cells = {(c, r) for r, row in enumerate(rows) for c, v in enumerate(row) if v == "1"}
    tops, sides = [], []
    for (c, r) in sorted(cells):
        x, y = x0 + c * s, y0 + r * s
        if (c, r - 1) not in cells:
            tops.append(f"M{x:g},{y:g}h{s:g}l{d:g},{-d:g}h{-s:g}z")
        if (c + 1, r) not in cells:
            sides.append(f"M{x + s:g},{y:g}l{d:g},{-d:g}v{s:g}l{-d:g},{d:g}z")
    return "".join(tops), "".join(sides), outline(cells, s, x0, y0), len(rows[0])


# -------------------------------------------------------------- the mark --


def mark(cx, cy, u):
    """Massif's mark: five blocks in an M, as the app's icon has them,
    back to front: (top, left, right, delay)."""
    ax = (u * math.cos(math.radians(30)), u * 0.5)
    az = (-u * math.cos(math.radians(30)), u * 0.5)

    def p(x, y, z):
        return "{:.1f},{:.1f}".format(cx + x * ax[0] + z * az[0], cy + x * ax[1] + z * az[1] - y * u)

    # (x, z, height, delay): the two legs, the saddle, the two steps.
    blocks = [(0, 2, 3.4, 0.0), (2, 0, 3.4, 0.0), (2, 2, 2.0, 0.18), (1, 3.2, 1.0, 0.32), (3.2, 1, 1.0, 0.32)]
    out = []
    for (x, z, h, delay) in sorted(blocks, key=lambda b: b[0] + b[1]):
        top = f"M{p(x, h, z)} L{p(x + 1, h, z)} L{p(x + 1, h, z + 1)} L{p(x, h, z + 1)}Z"
        left = f"M{p(x, h, z + 1)} L{p(x + 1, h, z + 1)} L{p(x + 1, 0, z + 1)} L{p(x, 0, z + 1)}Z"
        right = f"M{p(x + 1, h, z)} L{p(x + 1, h, z + 1)} L{p(x + 1, 0, z + 1)} L{p(x + 1, 0, z)}Z"
        out.append((top, left, right, delay))
    return out


# -------------------------------------------------------------- the card --


def svg(theme, at=None):
    dark = theme == "dark"
    cycle = SLOT * len(SCENES)
    tag = "#dbe3ee" if dark else "#334155"
    halo = "#%02x%02x%02x" % (NIGHT if dark else PAPER)
    border = "rgba(255,255,255,0.10)" if dark else "rgba(15,23,42,0.10)"
    shadow = "0.45" if dark else "0.16"

    def anim(name, dur, delay, extra=""):
        # Frozen at `at` to look at it: the same animation, paused there.
        if at is None:
            return f"animation:{name} {dur}s {extra} {delay:.2f}s both"
        return f"animation:{name} {dur}s {extra} {delay - at:.2f}s both;animation-play-state:paused"

    a = SLOT / cycle * 100
    f = FADE / cycle * 100
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Massif: the map editor for Minecraft, written in Rust">',
        "<title>Massif</title>",
        "<style>"
        "@keyframes in{from{opacity:0}to{opacity:1}}"
        # A scene: in over FADE, shown for SLOT, out over FADE, then away.
        f"@keyframes show{{0%{{opacity:0}}{f:.3f}%{{opacity:1}}{a:.3f}%{{opacity:1}}{a + f:.3f}%{{opacity:0}}100%{{opacity:0}}}}"
        f"@keyframes boil{{0%,{100 / PHASES - 0.01:.2f}%{{opacity:1}}{100 / PHASES:.2f}%,100%{{opacity:0}}}}"
        "@keyframes twinkle{0%,100%{opacity:.12}50%{opacity:1}}"
        "@keyframes rise{0%{opacity:0;transform:translateY(30px) scaleY(.3)}60%{opacity:1;transform:translateY(-3px) scaleY(1.05)}100%{opacity:1;transform:none}}"
        "@keyframes drop{0%{opacity:0;transform:translateY(-40px)}55%{opacity:1;transform:translateY(4px)}75%{transform:translateY(-1.5px)}100%{opacity:1;transform:none}}"
        "@keyframes slide{from{opacity:0;transform:translateX(-14px)}to{opacity:1;transform:none}}"
        "@keyframes hover{0%,100%{transform:translateY(0)}50%{transform:translateY(-4px)}}"
        ".b{transform-box:fill-box;transform-origin:50% 100%}"
        ".l{transform-box:fill-box;transform-origin:50% 50%}"
        "@media (prefers-reduced-motion:reduce){*{animation:none!important}}"
        "</style>",
        "<defs>",
        f'<clipPath id="card"><rect width="{W}" height="{H}" rx="{RADIUS}"/></clipPath>',
        '<linearGradient id="face" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fb923c"/><stop offset="1" stop-color="#ea580c"/></linearGradient>',
        f'<filter id="lift" x="-20%" y="-20%" width="140%" height="160%"><feDropShadow dx="0" dy="7" stdDeviation="7" flood-color="#000" flood-opacity="{shadow}"/></filter>',
        "</defs>",
        '<g clip-path="url(#card)">',
        f'<rect width="{W}" height="{H}" fill="{"#%02x%02x%02x" % (NIGHT if dark else PAPER)}"/>',
    ]
    # The scenes, one after another, each its four phases.
    rnd = random.Random(11)
    for k, (name, band_at, horizon, lift, sat) in enumerate(SCENES):
        phases, sky_mask = scene(name, band_at, horizon, lift, sat, theme)
        if at is None:
            style = f"animation:show {cycle:.1f}s linear {k * SLOT - FADE:.2f}s infinite both"
        else:
            t = (at + FADE - k * SLOT) % cycle
            style = f"opacity:{1 if 0 <= t < SLOT + FADE else 0}"
        parts.append(f'<g style="{style}">')
        for p, im in enumerate(phases):
            if at is None:
                ps = f"animation:boil {0.9 * PHASES:.1f}s steps(1) {-0.9 * (PHASES - 1 - p):.1f}s infinite"
            else:
                ps = f"opacity:{1 if p == int(at / 0.9) % PHASES else 0}"
            parts.append(f'<image href="{png_uri(im)}" width="{W}" height="{H}" preserveAspectRatio="none" style="image-rendering:pixelated;{ps}"/>')
        if dark:
            # Stars, where there is sky, away from the words.
            m = sky_mask.load()
            stars = []
            tries = 0
            while len(stars) < 30 and tries < 4000:
                tries += 1
                cx, cy = rnd.randrange(int(COLS * 0.45), COLS - 3), rnd.randrange(2, int(ROWS * 0.6))
                if all(m[min(COLS - 1, cx + i), min(ROWS - 1, cy + j)] for i in range(-2, 3) for j in range(-2, 3)):
                    size = rnd.choice([2, 2, 2, 3, 3, 4])
                    delay, dur = rnd.uniform(0, 5), rnd.uniform(2.2, 5.0)
                    st = f"animation:twinkle {dur:.1f}s ease-in-out {-delay:.1f}s infinite" if at is None else "opacity:.8"
                    stars.append(f'<rect x="{cx * CELL}" y="{cy * CELL}" width="{size}" height="{size}" fill="#e2e8f0" style="{st}"/>')
            parts.append("<g>" + "".join(stars) + "</g>")
        parts.append("</g>")

    # The mark, its blocks rising one after another.
    parts.append(f'<g filter="url(#lift)"><g style="{anim("hover", 6.0, 3.2, "ease-in-out infinite") if at is None else ""}">')
    for (top, left, right, delay) in mark(150, 172, 25):
        parts.append(
            f'<g class="b" style="{anim("rise", 0.9, 0.35 + delay, "cubic-bezier(.2,.8,.2,1)")}">'
            f'<path d="{left}" fill="{ORANGE}"/><path d="{right}" fill="{RUST}"/><path d="{top}" fill="{PEACH}"/></g>'
        )
    parts.append("</g></g>")

    # The name, in blocks, a letter after another.
    s, d = 15, 7
    x, y0 = 300, 142
    parts.append('<g filter="url(#lift)">')
    for i, ch in enumerate("MASSIF"):
        tops, sides, front, width = letter(ch, x, y0, s, d)
        parts.append(
            f'<g class="l" style="{anim("drop", 0.85, 0.85 + i * 0.09, "cubic-bezier(.2,.75,.25,1)")}">'
            f'<path d="{sides}" fill="{RUST}"/><path d="{tops}" fill="{PEACH}"/><path d="{front}" fill="url(#face)" fill-rule="evenodd"/></g>'
        )
        x += (width + 1) * s
    parts.append("</g>")
    # What it is.
    parts.append(
        f'<g style="{anim("slide", 0.8, 1.65, "ease-out")}">'
        f'<text x="302" y="{y0 + 7 * s + 46}" fill="{tag}" stroke="{halo}" stroke-width="5" stroke-opacity=".75" paint-order="stroke" stroke-linejoin="round" '
        f'font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" font-size="17" font-weight="600" letter-spacing="3.4">THE MAP EDITOR FOR MINECRAFT · WRITTEN IN RUST</text></g>'
    )
    parts.append("</g>")
    parts.append(f'<rect x=".5" y=".5" width="{W - 1}" height="{H - 1}" rx="{RADIUS}" fill="none" stroke="{border}"/>')
    parts.append("</svg>")
    return "".join(parts)


def main():
    args = sys.argv[1:]
    at = float(args[args.index("--at") + 1]) if "--at" in args else None
    out = args[args.index("--out") + 1] if "--out" in args else OUT
    if "--dawns" in args:
        # The light theme's pictures of the home screen, beside the night's.
        where = args[args.index("--dawns") + 1] if len(args) > args.index("--dawns") + 1 else BACKDROPS
        for (name, _, horizon, lift, sat) in SCENES + MORE:
            path = os.path.join(where, f"{name}_aube.jpg")
            dawn(name, horizon, lift, sat).save(path, "JPEG", quality=88, optimize=True, progressive=True)
            print(f"{path}: {os.path.getsize(path) // 1024} KiB")
        return
    for theme in ("dark", "light"):
        path = os.path.join(out, f"hero-{theme}.svg")
        with open(path, "w") as f:
            f.write(svg(theme, at))
        print(f"{path}: {os.path.getsize(path) // 1024} KiB")


if __name__ == "__main__":
    main()
