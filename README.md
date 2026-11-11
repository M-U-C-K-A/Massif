# Massif

Massif is a map editor for Minecraft Java Edition, written in Rust. It is a
modern successor to [WorldPainter](https://www.worldpainter.net/): you shape
and paint huge maps from above, sculpt real 3D features where you need them,
preview everything in 3D, and export a world Minecraft can open.

It is built for maps tens of thousands of blocks wide: no garbage-collector
pauses, a few hundred megabytes of memory for a 16k × 16k map, and work spread
over every CPU core.

> Status: early development. Worlds, tools and file format work end to end
> and are covered by tests, but some of WorldPainter's features (layers in
> particular) are still on the way. See the [roadmap](#roadmap).

## Highlights

- **Multi-level surface.** A dimension is a stack of levels: the ground
  height map, *solid* levels that add matter (overhangs, arches, floating
  islands) and *carve* levels that remove it (caves, tunnels, pits). All of
  them are edited with 2D brushes.
- **True 3D where it matters.** Build, Dig and Smooth (3D) sculpt sparse
  16³ voxel chunks that override the levels only where you sculpt. The 3D
  smooth works in every direction: cliff edges round off, sharp hollows fill
  in, flat faces stay where they are.
- **WorldPainter brushes and controls.** The twelve symmetric brushes with
  WorldPainter's profiles, 22 generated terrain stamps (peaks, ranges,
  volcanoes, mesas…), custom greyscale-image brushes (your WorldPainter
  brushes folder is read as is), radius up to 4,096 blocks, brush filters
  (height band, only on / except on, slope, curvature) and WorldPainter's
  mouse and keyboard bindings.
- **Terrain graphs.** Build terrain from nodes, as in World Machine or Gaea:
  noise, cells, gradients, levels, terraces, warp, blur, hydraulic and
  thermal erosion, combiners, height and slope selectors, and outputs that
  write heights and terrains to the map. Every node shows a live preview.
- **The game's real blocks.** Every block of the Minecraft version you have
  installed (1,288 in 26.3) is read from the game itself, with its real
  textures and colours: terrains show the actual blocks, and any whole block
  can be painted on the map.
- **Full-detail 3D view.** Every block column around the camera is meshed at
  full resolution; the rest of the map is drawn as a smooth relief, so the
  whole map is always in view.
- **WorldPainter import.** `.world` files are read directly (no JVM) and
  converted in about a second for a 100 MB world.
- **Minecraft export.** Region files and `level.dat` in the 1.18 chunk layout,
  which every later version opens and upgrades.
- **A compact, robust file format.** `.massif` stores each 128 × 128 tile as
  an independent, checksummed zstd frame: saving copies bytes, opening decodes
  nothing, corruption is detected and contained.

## Getting started

Massif needs a Rust toolchain. The exact version is pinned in
`rust-toolchain.toml` and `rustup` installs it automatically.

```bash
cargo run --release -p massif-app
```

Open a world directly (a WorldPainter `.world` file is imported the same way):

```bash
cargo run --release -p massif-app -- path/to/world.massif
```

The command-line tool creates, inspects, verifies, imports and exports worlds
without opening the editor:

```bash
cargo run --release -p massif-cli -- new world.massif --size 125x125 --landform mountains
cargo run --release -p massif-cli -- info world.massif
cargo run --release -p massif-cli -- verify world.massif
cargo run --release -p massif-cli -- import old.world world.massif
cargo run --release -p massif-cli -- export world.massif ~/Library/Application\ Support/minecraft/saves/MyWorld
```

## Using the editor

### Areas and workspaces

The window is made of areas, as in Blender. Each area shows any editor: the
map, the 3D view, tools, tool settings, brush settings, brushes, custom
brushes, terrain and biome palettes, levels or world info.

- The ⋮ menu of an area splits it, joins it with its neighbour, maximises it
  (Ctrl+Space) or closes it. Hold Shift for a one-click close button.
- Drag the top-right corner of an area into it to split it where you let go.
- Drag an area by its title onto another to move it, or onto its centre to
  swap the two. Drag the gaps between areas to resize them.
- Workspaces (Layout, Sculpting, Painting, 3D, Maps, Generate, Minimal) are tabs of
  arrangements; duplicate one with `+`. The layout is remembered.

### New worlds and export

**File ▸ New** asks for the name, seed, size (in multiples of 128 blocks),
the topography (flat, plains, hills, mountains, islands) with its relief
height and feature size, the base level, the water level, lava instead of
water, beaches, and either automatic terrains (grass, sand, rock and snow
by height and slope) or a single terrain; a preview of the whole map
updates as you change them.

**File ▸ Export** writes the world to a Minecraft saves folder with its
game mode, difficulty, cheats, hardcore, structures beyond the map, world
border, spawn point, what surrounds the map (vanilla terrain or void) and
data packs (copied in and enabled). **Test Export** writes only the 3 × 3
tiles around the centre of the map view to a separate folder, to check the
result in the game quickly.

### Brushes

Brushes work as in WorldPainter. A brush has a shape (one of the twelve
built-in brushes, a terrain stamp, or an image), a radius, an intensity and
a rotation:

- **Built-in brushes**: circle or square with a Constant, Linear, Sine,
  Plateau, Spike or Dome profile.
- **Terrain stamps**: generated height-map brushes for landforms; use them
  with Raise to stamp a peak, with Lower to carve a valley.
- **Custom brushes**: greyscale images (white paints, black does not),
  stretched over the brush. Massif reads its own `brushes` folder (every
  sub-folder is a group) and WorldPainter's brushes folders, in place.

Brush operations run tile by tile in parallel, so even a brush 8,000 blocks
across stays responsive and light on memory.

**Brush options** (in Brush Settings) limit where every brush acts, from
what the map shows:

- **Height limits**: at or above, at or below, between or beyond two
  heights, optionally fading over 4 blocks. A cross-section shows where the
  brush acts; drag its limits.
- **Only on / except on**: terrains, biomes, water or dry land.
- **Slope**: above or below an angle.
- **Curvature**: hollows or bumps, deeper than a number of blocks compared
  with the columns around.

### Real blocks

Massif reads the blocks of the game from your own installation: the client
jar of a released version, found where the official launcher, CurseForge or
Prism keep them (on any drive), or chosen by hand. Nothing of the game is
copied into Massif or the repository; the jar is read in place, in a
fraction of a second, every time Massif starts.

- Every block state file gives a block; its model, followed through its
  parents, gives the textures of its top, sides and bottom; grass, leaves
  and water are tinted as in the game (plains colours).
- The blocks and terrains of the open world take the real colours of their
  textures, on the map and in the 3D view.
- The **Terrain** panel draws each terrain with its main block's textures.
- The **Blocks** panel lists every block of the game by family (soil,
  stone, ores, wood, plants, building blocks, coloured blocks, Nether,
  End), with a search field and the version to read from. Clicking a whole
  block makes it a terrain of the world (grass, podzol and mycelium over
  dirt, as in the game; other blocks a few deep over the natural rock) and
  paints it.

### Terrain graphs

The **Terrain Graph** editor (in the Generate workspace) builds terrain the
way World Machine and Gaea do. Right-click the canvas to add nodes, drag
from an output to an input to wire them, and select a node to edit it and
see a larger preview. Every node previews the whole map as you edit.

| Family | Nodes |
|---|---|
| Generators | Noise (rolling, ridged, billow), Cells (cones, ridges), Radial gradient, Constant |
| Modifiers | Levels, Terraces, Clamp, Blur, Warp |
| Erosion | Erosion (simulated raindrops; outputs the heights, the water flow and where soil settled), Thermal erosion (material slides below a talus angle) |
| Combiners | Combine (add, subtract, multiply, max, min, average), Blend through a mask |
| Selectors | Select height, Select slope, Invert |
| Outputs | Height output, Terrain output (paints a terrain through a mask) |

**Build and apply** evaluates the graph over the whole map at the chosen
resolution (512 to 4,096 samples per side) on another thread, then writes
the heights and terrains to the ground as one undoable step. Distances are
in blocks, so a graph looks the same at every resolution. The graph is saved
with the world. A starter graph (eroded mountains, rock on steep slopes,
snow on the peaks, gravel where the water left soil) shows how it works.

### Controls

Massif follows WorldPainter's defaults.

| Input | Action |
|---|---|
| Left / right button (or Alt + left) | Apply the tool / its inverse (lower, erase, drain…) |
| Scroll | Brush size (Shift: one block at a time) |
| ⌘, Ctrl or Alt + scroll, pinch | Zoom the map |
| Middle button, Space + drag | Move the map |
| `−` `+` | Brush size |
| `1` … `9`, `0` | Intensity 10 % … 100 % |
| `[` `]` | Turn the brush |
| `R` `L` | Turn the light of the map |
| ⌥H ⌥A ⌥S ⌥P ⌥R ⌥F ⌥Y | Height, Flatten, Smooth, Paint, Spray, Flood, Eyedropper |
| ⌘Z / ⇧⌘Z, ⌘S, ⌘O, ⌘N, ⌘E | Undo / redo, save, open, new, export |
| Esc | Cancel the stroke being drawn |

In the 3D view:

| Input | Action |
|---|---|
| Left drag / right drag | Turn / move the view |
| Scroll, pinch | Come closer / go back |
| W A S D or arrows, Q / E | Fly over the map, down / up (Shift: faster) |
| Double-click | Look at the point under the cursor |
| With Build, Dig, Smooth (3D) | Left sculpts where the cursor points, right inverts; middle drag or Alt + drag turns, Shift + middle drag moves |

## How it works

### The world model

Each level of a dimension is a sparse grid of tiles of 128 × 128 columns.
Every column stores a top height (1/256 of a block), a bottom height for slab
levels, a terrain, a fluid level and a biome. Composing the levels of a column
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

### Sculpted volumes

Where you sculpt in 3D, the dimension keeps a 16³ chunk of densities (0–255,
solid from 128) that replaces what the levels compose inside that cube. A new
chunk starts as an exact copy of the composition, with the fractional surface
height as the density of the top blocks, so sculpting never changes anything
outside the brush. Chunks are shared copy-on-write, which makes undo cheap.
Sculpted matter open to the sky takes the terrain of the column's surface;
under something else it is rock.

### The `.massif` format

Little-endian and built for random access:

```text
header (64 bytes)   magic "MASSIF\x1a\n", format version, oldest reader version,
                    directory offset and CRC32C, file length, header CRC32C
tile frames         one zstd frame per tile, independent of the others
sections            META  world settings, registry and levels (zstd JSON)
                    TIDX  tile index of a level: x, z, offset, length, CRC32C
                    VOXL  sculpted chunks of a dimension (zstd)
directory           32 bytes per section: kind, id, offset, length, CRC32C, flags
```

Every tile and section has its own checksum. Unknown section kinds and tile
planes are skipped, so the format can grow without breaking older files or
older readers. Saving is atomic (a temporary file renamed over the target).

### WorldPainter import

A `.world` file is a gzip stream (whose header carries a JPEG thumbnail)
holding a Java `ObjectOutputStream` of a `World2`. Massif reads it with its
own implementation of the Java serialization protocol and converts tiles in
parallel as they are read. Heights, terrains (custom terrains are rebuilt
from their mixed materials), water, painted biomes, the Nether and the End,
seed, spawn and build limits are imported. Layers (trees, vegetation, frost,
caves…), lava and the ceiling, cave floor and floating dimensions are not
yet; the import report lists what was left out.

### Minecraft export

The exporter writes `region/r.X.Z.mca` and `level.dat` in the chunk layout
introduced in Minecraft 1.18 (DataVersion 2975, 1.18.2); every later version
opens these worlds and upgrades them on load. Regions are built in parallel
(4,096 chunks in 0.15 s). Around the map, Minecraft generates either vanilla
terrain from the world seed or nothing. The tests read the exported regions
back with an independent Anvil reader and compare every block with the model.

### The 3D view

The view keeps one mesh per tile on the GPU, rebuilt only where the world
changed, nearest first, within a time budget per frame. Faces of the same
height and colour are merged and drawn as 12-byte instances. On a mountainous
100 MB WorldPainter world, a 768-block view takes 700k faces built in 24 ms;
2,048 blocks take 6.5M faces (77 MB) in 0.16 s. Beyond the detailed area, a
smooth relief sampled every 16 blocks shows the rest of the map.

## Project layout

| Crate | Purpose |
|---|---|
| `massif-core` | The world model: coordinates, RLE planes, tiles, levels, column composition, voxel chunks, brushes, stamps and filters, editing operations, undo, terrain generation and terrain graphs. No graphics dependency. |
| `massif-format` | Reading and writing `.massif` files. |
| `massif-anvil` | Minecraft Java export: NBT, region files, `level.dat`. |
| `massif-legacy` | WorldPainter `.world` import, with a Java serialization reader written in Rust. |
| `massif-assets` | The game's real blocks: finds the installed client jars, reads their block states, models and textures (with its own small zip reader), real colours and textured block icons. |
| `massif-app` | The editor (egui and wgpu): areas, map, 3D view, panels. |
| `massif-cli` | The command-line tool. |

## Testing

```bash
cargo test --workspace
```

Two environment variables enable the heavier tests:

- `MASSIF_SNAPSHOT_DIR=<folder>` renders the whole window offscreen (with
  egui_kittest and wgpu), the map, the 3D view, the brush panels, the
  dialogs, the terrain graph and the real blocks to PNG files in that
  folder, and drives the editor with simulated mouse strokes, 3D sculpting,
  area splitting, a terrain graph build and painting a real block. The real
  blocks are only tested when a Minecraft installation is found.
- `MASSIF_WORLD_SAMPLES=<file.world>` also imports a real WorldPainter world
  through the editor and measures the 3D meshing of it.

## Roadmap

- WorldPainter layers (trees, vegetation, ores, frost, roads…) and their
  import from `.world` files, and layers as brush filters and graph outputs.
- Terrain graphs: more nodes (rivers, sediment layers, height-map import,
  stamps), graphs applied to a part of the map, tiled builds beyond 4,096
  samples, a 3D preview of a node.
- The real blocks' textures in the 3D view, and custom terrains mixing
  real blocks in layers.
- Custom build limits and older Minecraft versions in new worlds and export.
- Selections, copy and paste, height-map import.
- Nether and End editing from the editor, customisable key bindings.

## License

Massif is declared GPL-3.0-or-later in `Cargo.toml`, like WorldPainter.
