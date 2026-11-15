# The README's figures

## The banner

`hero-dark.svg` and `hero-light.svg`, at the top of the README, are a card of
the home screen's five night scenes, one after another, dithered as Massif's
home screen dithers them — at night for GitHub's dark theme, at dawn for the
light one (each scene's `_relief` layer lifted into daylight under a sky drawn
for it) —, the dither moving in four phases; stars twinkle in the night skies.
Massif's mark of blocks rises over it, and its name falls into place in
blocks, a letter after another. CSS animations only, so they play inside an
`<img>`; the pictures are PNGs inside the SVG.

```bash
python3 docs/assets/src/hero.py            # both themes, into docs/assets
python3 docs/assets/src/hero.py --at 1.2   # frozen 1.2 s in, to look at it
```

It needs Pillow, and reads the scenes from `crates/massif-app/assets/backdrops`.

## The other figures

The animated figures of the README, drawn as isometric line drawings in the
manner of [Hairline](https://lucasmarkes.com/lab/hairline): rounded solids, a
silhouette and one crease each, one bright stroke. Every figure comes in two
files, `<name>-light.svg` and `<name>-dark.svg`; the README picks one with
`<picture>` and `prefers-color-scheme`, as GitHub does for its own themes.

Their background is transparent: the solids are filled with the colour of
GitHub's page (white, or `#0d1117`) so that what is behind them stays hidden.
They move with SMIL animations alone, so they play inside an `<img>`.

| Figure | What it shows |
|---|---|
| `mark` | Massif's voxel peak rising out of its slab, once |
| `brush` | A brush raising a ridge across a massif, then the stroke undone |
| `islands` | Two islands of rock floating over a field of columns |
| `graph` | Pulses running through a terrain graph to its output |
| `rivers` | A river found from its source down a valley to the sea |
| `blocks` | Six of the game's blocks, picked one after another |

## Drawing them again

```bash
node docs/assets/src/build.mjs
```

The geometry comes from the Hairline kernel (`kernel.js` of the
`hairline-create` skill, read from `~/.claude/skills/hairline-create/` or from
the file `HAIRLINE_KERNEL` names). `build.mjs` writes into `docs/assets`; give
it another folder, and figure names, to draw only some.

To look at a figure frame by frame, `sheet.mjs` lays both themes side by side,
paused at the times given, and prints where the picture is (it needs
`playwright-core` and Chrome):

```bash
node docs/assets/src/sheet.mjs rivers 0,2,3,5
```
