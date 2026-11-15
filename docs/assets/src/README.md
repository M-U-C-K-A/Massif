# The README's figures

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
