# How Massif works

[← Back to the README](../README.md)

The world model, sculpted volumes, the `.massif` format, WorldPainter import,
Minecraft export, the 3D view and very large maps.

## The world model

Each level of a dimension is a sparse grid of tiles of 128 × 128 columns.
Every column stores a top height (1/256 of a block), a bottom height for slab
levels, a terrain, a fluid level, a biome, a ground cover and an object
layer. Composing the levels of a column
yields a short list of solid spans along Y, which is a run-length encoding of
the column: the exporter turns spans into blocks, the 3D view into faces, the
map shows the top one.

Each plane of a tile is uniform (no allocation), run-length encoded (split
streams, O(log n) lookup without decompression) or dense, and is compacted
back after edits. Tiles are either *hot* (decoded and editable) or *cold*
(one compressed blob, the exact bytes the file stores); only recently touched
tiles stay decoded. A 16,000 × 16,000 mountain map generates in 5.6 s, takes
182 MB on disk (0.74 byte per column), and the tool that generates and saves
it peaks at 236 MB of memory.

## Sculpted volumes

Where you sculpt in 3D, the dimension keeps a 16³ chunk of densities (0–255,
solid from 128) that replaces what the levels compose inside that cube. A new
chunk starts as an exact copy of the composition, with the fractional surface
height as the density of the top blocks — and read back from them, to the
1/255 of a block —, so sculpting never changes anything outside the brush. Chunks are shared copy-on-write, which makes undo cheap.
Each sculpted block can keep a terrain of its own: matter pushed out (an
extruded ledge, an overhang, a bump) takes the terrain of the blocks it was
pushed from, so a sand wall gives a sand ledge, while the ground left under
it keeps its own — nothing is stretched from the ground up. 3D **Paint**
repaints sculpted matter too. Matter that keeps no terrain of its own
follows the column: the terrain of its surface where it is open to the sky,
rock under something else.

## The `.massif` format

Little-endian and built for random access:

```text
header (64 bytes)   magic "MASSIF\x1a\n", format version, oldest reader version,
                    directory offset and CRC32C, file length, header CRC32C
tile frames         one zstd frame per tile, independent of the others
sections            META  world settings, registry and levels (zstd JSON)
                    TIDX  tile index of a level: x, z, offset, length, CRC32C
                    VOXL  sculpted chunks of a dimension (zstd): densities,
                          and the terrain of each block that keeps one
                    THMB  a JPEG picture of the map, for the Finder
directory           32 bytes per section: kind, id, offset, length, CRC32C, flags
```

Every tile and section has its own checksum. Unknown section kinds and tile
planes are skipped, so the format can grow without breaking older files or
older readers. Saving is atomic (a temporary file renamed over the target).

## WorldPainter import

A `.world` file is a gzip stream (whose header carries a JPEG thumbnail)
holding a Java `ObjectOutputStream` of a `World2`. Massif reads it with its
own implementation of the Java serialization protocol and converts tiles in
parallel as they are read. Heights, terrains (custom terrains are rebuilt
from their mixed materials), water, painted biomes, the Nether and the End,
seed, spawn and build limits (worlds WorldPainter made taller than the
game, up to 2032 blocks, keep their height) are imported. WorldPainter's
Beaches (and Bare Beaches) come in as it draws them — grass with patches of
sand and gravel, big ones, and of clay, small ones, from noise of its own
scales — rather than sand all over. Layers (trees, vegetation, frost,
caves…), lava and the ceiling, cave floor and floating dimensions are not
yet; the import report lists what was left out.

## Minecraft export

The exporter writes `region/r.X.Z.mca` and `level.dat` for one version of
the game. The blocks of a chunk are worked out once, then stored the way
that version does:

| Versions | Chunks |
|---|---|
| 1.7.10 – 1.12.2 | Numbered blocks (an id and four bits of data), a biome per column, the light of the sky worked out; 256 blocks high |
| 1.14 – 1.15 | A palette of named block states per section, indices packed end to end; biomes numbered, in 4 × 4 × 4 cells from 1.15 |
| 1.16 – 1.17 | The same, indices no longer straddling two longs |
| 1.18 – 26.3 and later | Today's layout: palettes of blocks and of biomes, −64 to 320, or the map's own height |

Blocks and biomes a version does not have yet become the closest it has
(deepslate is stone before 1.17, moss is grass, mud is coarse dirt…).
Ground covers are grown on the top of each column, after its blocks and its
water: in the air above dry ground, in the water for those that grow there,
on soil only for those that need it, where the whole plant has room. Then
the objects that reach into the chunk, from its tile or the next ones, are
stood in it in a fixed order, so that where two meet the same one wins on
both sides of a chunk border.
Versions newer than the built-in list are taken from the game itself when
it is installed (its jar says which data version it writes). `level.dat`
keeps the shape of 1.18.2 for every later version, which bring it up to
date when they open the world; 1.13 is left out (it lights chunks in a way
of its own). Regions are built in parallel (4,096 chunks in 0.15 s).
Around the map, Minecraft generates either vanilla terrain from the world
seed or nothing. A map taller (or deeper) than the game's −64 to 320 gets
its sections up to its top and a data pack, `massif_build_limits`, enabled
in `level.dat`, that gives the overworld that height (Minecraft allows −2032
to 2032; the data pack's format follows the version). The tests read every era's regions back with an
independent Anvil reader and compare the blocks with the model.

## The 3D view

Most columns of a map are one span of ground on the bottom of the world: a
height field. The view keeps those in GPU textures, pages of 1024 × 1024
columns holding each column's height and colours in 4 bytes, with a max
pyramid (the highest column of every 2 × 2, 4 × 4… square). A ray tracer in
the fragment shader walks down the pyramid, skipping the air above the
ground, and stops on a column, or on a square smaller than a pixel: the whole
map is drawn block by block at any distance, with no level of detail and no
"view distance". The other columns (overhangs, floating islands, caves open
to the side) and the water are meshed per tile, with faces of the same height
and colour merged into 12-byte instances; tracer and meshes share the depth
buffer. Tiles are rebuilt only where the world changed, nearest first.

On a mountainous 100 MB WorldPainter world of 6,144 × 6,144 blocks, the whole
map is built in about a second and takes 176 MB of GPU memory; meshing it
instead would have taken some 58 million faces.

The player's view in 3D Sculpt draws the game's look (`textures.rs`). Every
face and every traced column also says what it is made of: a *material*,
the registry's block index, or after them an entry of an object's palette;
a column's colour pair names its top block and its soil, and the free top
byte of a packed column holds the depth of water over it. From the client
jar the Blocks panel reads, an atlas gathers the textures of every material
(top, side and bottom, a log's axis) in one 16 × 16 array with its levels
of detail, and the shaders sample it by the face's direction and height
(the rock under the soil turns to deepslate under y 0). They light it as
the game does — its shade per face in its gamma, the smooth lighting's
corners from the eight columns around, a light level a block under water —
and draw its sky, square sun, clouds (the jar's `clouds.png`, four blocks
thick at y 192, crossed cell by cell) and fog. The brush's blocks and the
block looked at are marked in the shaders too, block by block. The game's
look costs about 0.4 ms a frame at 1,600 × 1,000 on an M4 (2.3 ms → 2.7).

The views' own look (the 3D View and 3D Sculpt, Terrain) samples the same
atlas, in its own light, with the corners' shade of the game's smooth
lighting. Each look is a shader of its own — flat colours, textured, the
game's — so that Low pays nothing for textures it does not draw. A top
takes the tint of what grows on it: the colour pair's top over the block's
bare colour, and for meshed faces the face's colour over the material's own
(a table beside the atlas). Far off, a square of the max pyramid smaller
than a pixel takes the mean of its columns' textures (their last level of
detail, which is what a texture fades to up close), so the near ground and
the far meet without a seam. On a map of 16,384 × 16,384 blocks at
2,560 × 1,440 (M4, best of 12), textures cost 0.5 to 1.9 ms a frame while
the view moves (Medium 5.2–6.4 → 5.7–7.5 ms, High 11.0–14.2 → 12.9–16.2),
and 1 to 4 ms on the frame drawn once, a ray a sample, when it stops; the
atlas takes 0.1 to 0.3 MB of the card's memory.

Every block of a 60,000 × 50,000 map would take some 13 GB of textures.
Past 400 pages (about 1.9 GB), the view keeps a column every 2, 4, 8 or 16
blocks — the coarsest that fits — and still draws the whole map: each of
its tiles holds that many tiles of the map, their water drawn as a surface,
and the rays are walked in its coarser grid. The header says so. A map of
24,000 × 24,000 blocks (35,000 tiles) is shown at a column every 2 blocks
in 672 MB, built in 25 seconds, nearest the camera first.

### On OpenGL

wgpu draws with OpenGL where neither Vulkan nor DirectX 12 can (an Intel HD
Graphics of 2013, a graphics API set aside after a stuck start), and OpenGL
does less. The views draw in sRGB, which egui shows as plain bytes: where a
texture cannot be viewed in another format, a small pass copies the image
into a texture of its own, the same bytes (`view_image.rs`). The
multisampled image is kept until it is resolved (wgpu's OpenGL drops it
first, and the image came out empty), a texture read as an array has two
layers at least (OpenGL takes one of a single layer for a plain 2D one),
the fragments' input carries no `@invariant`, which OpenGL ES refuses, and
an image read back is waited for again when OpenGL gives up waiting.

The tests run on OpenGL on macOS too, through ANGLE: the test build has
wgpu's `angle`, and ANGLE's `libEGL.dylib` and `libGLESv2.dylib` (Chrome's
or Firefox's ship them) only need to be found:

```
WGPU_BACKEND=gl DYLD_LIBRARY_PATH=<their folder> cargo test -p massif-app -- --test-threads=1
```

One at a time: ANGLE crashes with several OpenGL contexts at once.

## Very large maps

Tiles are kept compressed (cold) and decoded when they are needed, so a
map costs its compressed size. Measured on a hilly map of 60,000 × 50,000
blocks (183,379 tiles): generated in 57 seconds on 10 cores, 1.2 GB
compressed and 1.4 GB of memory at most; a brush 400 blocks across applies
in 2 ms; the whole map read by a graph's Map node, at 512 × 512, in 2
seconds. WorldPainter worlds are imported as a stream: each tile is
converted and freed as it is read.

## Areas, floating panels and the Render side

A workspace (`layout::Workspace`) is a tree of areas plus `floating`, the
panels taken out of it: each keeps its area id (so a map view or a 3D view
keeps its state), its editor, and its place and size in points. They are
drawn after the tree, each in an `egui::Area` of the middle order over the
central panel, through the same `area()` as a docked area (its header, its
menu, its editor), with a corner to size it. `Layout::detach` and
`Layout::dock_back` move an area between the two; floating panels are
serialised with the workspace, renumbered when it is duplicated and cleared
when it is reset. They live in the main window rather than in OS windows
(egui viewports): one wgpu renderer and one set of native textures (the 3D
views, the path tracer) serve them, and the kittest harness drives them.

Each workspace belongs to a side (`Workspace::side`: `render`, else
`terrain`, else the World side). The Render side's editors are
`RenderView`, `RenderSky`, `RenderCamera`, `RenderOutput` and `Timeline`,
all drawn from one `render_side::RenderSide`; only one area draws the
viewport in a frame (the first), the others say where it is.

### Camera paths

`massif_core::animation::Animation` (kept in `WorldMeta::animation`, so in
the `.massif` file) holds keyframes: a time in seconds and a `Pose` (eye,
target, vertical field of view, hour). `Animation::sample(t)` gives the pose
at any time: the eye and the target on a centripetal Catmull-Rom spline
through the keys (ends mirrored), the field of view and the hour eased
between neighbouring keys (the hour the short way round midnight). The
Timeline panel edits it; moving its playhead puts the Render side's orbit,
field of view and hour at `sample(playhead)`.

Left to do, in order:

- **Render a sequence**: for frame `n` of `frames()`, `sample(time_of(n))`,
  put it on the camera and the sun, draw a still to the samples asked (the
  still path of `RenderSide` does one), write `name_0001.png`…; progress
  and Cancel as for a still; then MP4/ProRes through `ffmpeg` when it is on
  PATH. The same loop in `massif-cli` for batch renders.
- **Speed along the path**: arc-length parameterisation, so that the
  camera moves at an even speed, with an ease curve per key.
- **Orientation keys**: roll, and orientation independent of the target
  (quaternion squad) for free-flying shots.
- **More animated values**: focus distance and aperture, day of the year,
  clouds' wind (their drift), exposure.
- **Undo** of keyframe edits, and a camera path drawn in the viewport
  and on the 2D map.
