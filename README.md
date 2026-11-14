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
- **A node-based terrain generator, in the manner of Gaea.** The Terrain
  side of the app: about sixty nodes — landforms (tectonic plates you move
  by hand, mountains, ranges, volcanoes, craters and crater fields, dunes,
  canyons, mesas, islands), primitives, adjustments, simulations (water and
  thermal erosion, snow, rivers traced from the sea to their sources, lakes,
  sea), masks, climate (temperature, humidity with its rain shadows, the
  game's biomes) and colour (gradients from nature, automatic colouring,
  tints), rock over the void (floating islands, overhanging cliffs), ten
  landscapes to start from (continents, archipelago, karst towers, floating
  mountains, river valley, desert canyon, volcanic island…) and two
  templates for the map you have (paint it, give it rivers) —
  rendered with a sun you turn, a sky that follows it, volumetric clouds,
  haze and a sea, saved as images up to 8K, then built at up to
  16,384 × 16,384 and converted to voxels: the colours become the closest
  real Minecraft blocks, the floating rock levels of the map.
- **The game's real blocks.** Every block of the Minecraft version you have
  installed (1,288 in 26.3) is read from the game itself, with its real
  textures and colours: terrains show the actual blocks, and any whole block
  can be painted on the map.
- **The whole map in 3D, every block.** The 3D view draws the entire map at
  block resolution, at any distance: a GPU ray tracer walks the height field
  of the ground (4 bytes per column), and the overhangs, islands, caves and
  water are meshed. It moves independently of the map view.
- **WorldPainter import.** `.world` files are read directly (no JVM) and
  converted in about a second for a 100 MB world.
- **Minecraft export, for the version you play.** From 1.7.10 to the
  newest release (26.3, and whatever is installed on the computer): each
  version gets the chunks it reads natively — numbered blocks up to 1.12,
  named ones from 1.14, today's layout from 1.18 — with only the blocks
  and biomes it knows.
- **Rivers.** Draw a river's course on the map — a few clicks — and it is
  dug with banks easing into the land and water that only runs down, as
  narrow as a brook or hundreds of blocks wide, branching from rivers and
  flowing into them; or let the generator trace them over a map: main
  rivers and their tributaries in the bottoms of the valleys, growing
  downstream, dug at the block, lakes in the hollows they cross.
- **Maps the size of WorldPainter's.** 60,000 × 50,000 blocks: generated
  in a minute, 1.2 GB compressed, edited at once, shown whole in 3D (a
  column every few blocks when every block would not fit the GPU).
- **Automatic biomes and paint.** Auto biome reads the land (deep water,
  rivers, beaches, plains, hills, deserts, snow, rock) and gives it its
  biomes; Auto paint gives each biome its ground, its cliffs and what lies
  under its water. Both with a brush or over the whole map, by rules you
  can change.
- **A compact, robust file format.** `.massif` stores each 128 × 128 tile as
  an independent, checksummed zstd frame: saving copies bytes, opening decodes
  nothing, corruption is detected and contained.

## Getting started

Massif opens on its **home screen**, not on a map made up for it: on the
left, the ways in — a new map, a map from a height map, a map of your own to
open (`.massif`), a WorldPainter world to import (`.world`) — and the maps
opened lately (or, at first, an empty list that says so); in the middle,
the settings of a new map with its preview, and **Create map**, laid on the
backdrop itself: ridges drawn in dots, lit by the accent colour, a few of
them twinkling now and then. A `.massif`, a
`.world` or a height map dropped on the window opens. **File ▸ New** (⌘N)
comes back to it, with a way back to the open map (Esc).

**Settings** (⌘, or the home screen's sidebar) choose the theme and the
accent colour: the colour of what is chosen, of the main buttons and of the
home screen's art (eight colours, or any other).

### Download

Builds for Windows, macOS (Apple silicon and Intel) and Linux are on the
[releases page](https://github.com/M-U-C-K-A/Massif/releases). They are not
signed: on macOS, right-click Massif.app and choose Open the first time (or
run `xattr -dr com.apple.quarantine Massif.app`); on Windows, choose More
info ▸ Run anyway if SmartScreen asks.

### From source

Massif needs a Rust toolchain. The exact version is pinned in
`rust-toolchain.toml` and `rustup` installs it automatically.

```bash
cargo run --release -p massif-app
```

On a Mac, `tools/install-macos.sh` builds it and installs **Massif.app** in
Applications (or the folder given), to open like any other app; run it again
after pulling changes to update it.

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
cargo run --release -p massif-cli -- export world.massif ~/Library/Application\ Support/minecraft/saves/MyWorld --version 1.21.11
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
- The app has two sides, switched at the top left: **World**, the map
  editor (workspaces Layout, Sculpting, Painting, 3D, 3D Sculpt, Maps,
  Minimal), and
  **Terrain**, the terrain generator (Graph and Render). Each side
  remembers the workspace it was left on.
- Workspaces are tabs of arrangements; duplicate one with `+`, right-click
  it to delete it. The layout is remembered.

### New worlds and export

A new map (the home screen) takes a name, a size (width × depth, in
multiples of 128 blocks, up to 65,536), its **height** — Minecraft's own (−64 to 320) or
taller, up to 512, 1024 or 2032 (the highest Minecraft allows), and as deep
as −2032 — its land — a
topography (flat, plains, hills, mountains, islands) with its relief height
(up to fifteen times the landform's in a tall world; peaks close in on the
top of the world rather than being cut flat), feature size and ground
level, or a template of the terrain generator (alpine range, continents,
archipelago…), whose map then opens on the Terrain side with its nodes —
the water level, lava instead of water (opaque and glowing, its shores
scorched basalt and blackstone, never sand), beaches, automatic terrains
(grass, sand, rock and snow by height and slope) or a single one, and a
seed; a preview of the whole map updates as you change them. A map made
from a height map takes a height too.

**File ▸ Export** writes the world to a Minecraft saves folder for the
version of the game you choose — the newest one installed on the computer
to begin with — with its game mode, difficulty, cheats, hardcore,
structures beyond the map, world border, spawn point, what surrounds the
map (vanilla terrain or void) and data packs (copied in and enabled).
Versions before 1.18 hold 256 blocks of height: the map is cut below 0 and
above 255, and the dialog says so. **Test Export** writes only the 3 × 3
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

The outline on the map is exact: zoomed in, it follows the blocks the brush
will change, block by block; zoomed out, a custom brush is outlined through
every pixel of its image. Inside it, the brush shows how hard it presses:
the map is lighter where it presses more, with a line where its strength
is a quarter, a half and three quarters — the rings of a soft round brush,
the peaks of a terrain stamp.

**Brush options** (in Brush Settings) limit where every brush acts, from
what the map shows:

- **Height limits**: at or above, at or below, between or beyond two
  heights, optionally fading over 4 blocks. A cross-section shows where the
  brush acts; drag its limits.
- **Only on / except on**: terrains, biomes, water or dry land.
- **Slope**: above or below an angle. Slopes are the trend of the hillside
  (a plane fitted over 9 × 9 columns), not the steps between single blocks;
  the map and 3D views' Slope mode use the same measure.
- **Facing**: only the slopes turned a given way, give or take an angle —
  the side the sun lights, the side in the shade. A compass sets the
  direction (north up, as on the map); **Sun** and **Shade** take it from
  the light of the map. Level ground faces nowhere. Snow on the northern
  slopes, moss in the shade, bleached rock in the sun: one stroke each.
- **Curvature**: hollows or bumps, deeper than a number of blocks compared
  with the columns around.

### Automatic biomes and paint

Two tools of the Paint group work from what the map already is, with a
brush (and its options) or over the whole map at once:

- **Auto biome** reads the land and gives it biomes. Every column is one
  of twenty kinds of land, told from its water (deep, shallow, or with a
  bank close on both sides: a river), its height over the sea (lowland,
  hills, mountains, peaks), its slope (cliffs are rock, whatever grows on
  them) and what its ground is made of (grass, sand, red sand, snow, moss,
  mud, mycelium, podzol, stone); sand at the water's edge is a beach, not
  a desert. Each kind has a biome.
- **Auto paint** goes the other way: every biome has a terrain for its
  ground, one for its cliffs and one for what lies under its water. Where
  no biome is set, the ground is painted as the land it is.

Hovered over the map, both show what they would change before anything is
painted: under the brush, the columns that would change in their new colour,
the others darkened. A stroke changes every column in the core of the brush
(all of it at 100% intensity, with a blended edge below), and says how many
it changed — or that the land already follows the rules.

**Rules…** (in Tool Settings, or Edit ▸ Automatic rules) opens both tables:
the depths, heights and slopes the land is read with, the biome of each
kind of land, the three terrains of each biome. They are kept with the
settings. Forests are told from the ground for now (podzol, coarse dirt);
trees will count when layers exist.

### Patterns of terrains

Paint terrain can lay several terrains at once (Tool Settings ▸ a pattern
of terrains): grass with patches of coarse dirt and podzol, stone veined
with gravel, a patchwork of fields. The pattern is a noise — **patches**,
**veins**, **blobs**, **cells** or wavering **stripes** at an angle — of the
size you choose, with soft or sharp edges and a seed; every terrain gets
the share of the ground its weight says, whatever the pattern (the pattern
learns its own spread and paints by rank). A picture shows it before the
first stroke, and the brush, its intensity and its options apply as with
any paint.

### Rivers

The **River** tool (Water group) lays a river out from its source:

- **Click** on land: the river springs there, and its water finds its own
  way down — down the slope, a block at a time; a hollow it meets fills up
  to where it overflows, and it goes on from there — to the sea, into water
  already there, or off the map. Before the click, hovering shows where the
  water would go.
- **Click elsewhere**: that is where it ends. The way there is worked out
  over the land, as water would take it: along the valleys, climbing as
  little as it can, into the water it meets.
- **Drag the river** anywhere along its course to bend it: the point held
  becomes a point it must pass, and the way on either side follows the land
  again. Drag the source, those points or the end to move them; right-click
  one to take it away (the end: back to where the water goes by itself).
- **Dig river** (or Enter) digs it, one undoable step; **Cancel** (or Esc)
  drops it. The bar over the map says how long it is and where it goes.

Its settings: **Width** at the source, **Flow** (how much it grows
downstream: up to five times wider and two and a half times deeper),
**Depth**, **Erosion** (how much it wears the land beside its bed: a wider
valley), **Banks**, **Lakes** (which hollows fill: deep ones only, or every
one, or none — through the others it cuts a gorge), the terrain of its bed
and a river biome on its water.

- The course is smoothed into a curve through its points.
- Water runs from the source down. Its level along the river is the lowest
  the land allows so far, a
  little under the banks: it never rises downstream, whatever the bumps.
- The bed is `depth` deep in the middle and rises to the water's edge;
  banks of the chosen width climb back to the ground, smoothly. Only the
  ground is lowered: nothing is raised.
- A course that starts on a river is a branch, leaving it at its level; one
  that ends on a river or a lake is a tributary, meeting it at its level.
  Two branches that meet again make a split stream.
- The bed gets a terrain (gravel, sand…), the water a river biome.

The water's level is a whole block per column, so it stands in steps
along the river: still water, as Minecraft keeps it; mods that let water
flow over water (Streams Reflowing) make a current of it.

### Height maps

- **File ▸ New World from Height Map…** makes a world of a greyscale image
  — a 16-bit PNG from Gaea, World Machine or an image editor (JPEG, and raw
  square `.r16` / `.r32` are read too): what height black and white stand
  for, how many blocks a pixel covers, the sea's level; terrains are painted
  from height and slope. The home screen offers it too.
- **File ▸ Export Height Map…** saves the map's surface (with its levels and
  what was sculpted) as a 16-bit PNG, black the bottom of the world, white
  its top.
- In a graph, the **File input** node brings in a file as relief: a height
  map (stretched over the terrain, black and white set to any height), a
  WorldPainter `.world` or a `.massif` (fitted to the terrain, or read at
  its own place). Its Height, Water and Colour outputs work like the Map
  node's, and the file is read again when it changes. **Height map…**
  beside Convert to voxels builds the terrain at full size and saves its
  relief as a 16-bit PNG instead of writing it to the map.

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

The Terrain side holds the generator. A graph makes its relief from
nothing — or starts from the map as it is, painted or imported from
WorldPainter, with the **Map** node (see below). Its **Graph** workspace is laid out
like Gaea: the **Graph Viewport** above, the **Terrain Graph** below, the
**Node Properties** on the right. The viewport shows the graph's output;
select a node and press **V** (or double-click it) to see the graph up to
that node — heights as a relief, masks in white over the relief they come
from, colours on it — and V again to go back to the output.

The **Render** workspace gives the viewport the stage, with the **Render
Settings** beside it, as when a Gaea terrain is rendered in Blender:

- the sun's direction, height and strength (Shift + drag in the viewport
  turns it), with soft cast shadows over the relief;
- a sky that follows it, blue by day, warm at sunset;
- volumetric clouds — cover, height, thickness, density, size, a seed to
  move them — lit by the sun and casting their shadows on the ground;
- haze with distance, a sea at the world's sea level, the exposure and the
  height exaggeration;
- **Save image** renders the view at full quality, from 1920 × 1080 to
  7680 × 4320, as a PNG. The view is drawn from the preview: a finer
  preview (up to 2,048 samples) gives a sharper image.

In the viewport, left drag turns the view, right or middle drag moves it,
the wheel or a pinch comes closer, a double-click frames the relief.

**Templates** (in the graph's toolbar) start from a ready-made landscape,
each an ordinary graph to change, shown with a picture of what it makes on
this map:

| Template | What it makes |
|---|---|
| Alpine range | Eroded peaks over green hills, snow on the summits (the starter graph) |
| Archipelago | Tropical islands in a shallow sea, sandy beaches, green and rocky peaks |
| Karst towers | A forest of stone towers, each of its own height, with sheer walls broken by ledges, scree at their feet, green tops and mossy walls, over a jungle crossed by rivers |
| Floating mountains | Mountains of rock floating over such towers, each of several masses with their undersides hanging to points: a level of rock over the void, above the mist |
| River valley | A wide valley between snowy ridged mountains, a river along its floor |
| Desert canyon | A winding, stepped canyon in a red plateau, buttes, a river at the bottom |
| Volcanic island | A volcano rising from the sea, black beaches, green lower slopes |
| Dune sea | Long dunes shaped by the wind, ripples over them |
| Rolling hills | Meadows, streams and ponds |
| Continents | Continents and oceans from tectonic plates, ranges where they collide, rivers to the sea, and climates from tundra to jungle written as the game's biomes |
| Paint this map | The map as it is, painted from its relief: grass, worn ground on the steeper slopes, rock on the cliffs, gravel where water runs, sand on the shores, snow on the heights |
| Rivers on this map | The map as it is, with rivers traced in its valleys and lakes in its hollows |

Templates follow the world: their sea is the world's sea level, and their
features grow with the map. Each also sets the sun, the clouds and the sea
of the viewport to suit it. "Back to the previous graph" undoes the choice.

Right-click the graph or press Tab to open the node library and type to
search; drop a wire on empty space to add a node already wired. Ports and
wires are coloured by what they carry: heights, masks or colours.

- Click a node to select it; Shift or ⌘ adds nodes to the selection, and a
  box dragged on the background selects all it touches. Dragging a node
  moves the whole selection.
- ⌘D duplicates the selection, ⌘C / ⌘X / ⌘V copy, cut and paste it with its
  wires (between worlds too), Delete removes it, ⌘A selects everything, Esc
  nothing; right-click (or Alt-click) a wire to cut it.
- Right-drag, middle-drag or Space + drag moves around; a mouse wheel or a
  pinch zooms, two fingers on a trackpad move; F frames the selection (or
  everything).
- ⌘Z / ⇧⌘Z on the Terrain side undo and redo the changes to the graph —
  and the conversions, in the order they were made.
- A line along the bottom of the graph says what can be done with what is
  selected.
Previews are evaluated on another thread with a cache, so changing a
setting computes only that node and what follows it.

| Family | Nodes |
|---|---|
| Landforms | Tectonic plates (height, ranges, faults), Mountain (alpine, rounded, eroded, spiky), Mountain range, Volcano, Crater, Crater field, Dunes, Canyon, Mesas, Island, Rocks, Floating islands (top, bottom, mask) |
| Primitives | Noise (rolling, ridged, billow), Cells (cones, ridges, or one height per cell), Radial and linear gradients, Shape, Constant |
| Adjust | Levels, Normalize, Curve, Terraces, Stratify, Clamp, Blur, Sharpen, Warp, Transform, Swirl, Fold, Invert height, Overhang (the rims of cliffs over the void) |
| Simulate | Erosion (heights, flow, deposits, wear), Thermal erosion (heights, talus), Snow, Rivers, Lakes, Sea (water and shore masks) |
| Combine | Combine (add, subtract, multiply, max, min, average, lerp, screen, overlay, difference), Blend through a mask |
| Masks | Select height, Select slope, Curvature (ridges or hollows), Facing, Occlusion, Flow, Invert |
| Climate | Temperature, Humidity, Biomes (the colours of the climate) |
| Colour | Auto colour (eight kinds of land), Gradient (twenty gradients from nature), Solid colour, Mix colours, Tint, Adjust colour |
| Output | Height output, Terrain output, Colour output, Water output, Layer output (rock between two heights, over the ground), Biome output |

A height map cannot hold an overhang or an island in the air; the map
can, its levels being spans of rock stacked in each column. **Layer
output** is the way from one to the other: it takes a top and a bottom —
from **Floating islands**, from **Overhang**, or from any two heights —
and optionally the colour of the top. The viewport draws up to two layers
over the relief, lit, casting their shadows on the ground and on each
other; viewing one of these nodes (V) shows it over the graph's ground.
Converted, each layer becomes a Solid level of the map named after what
makes it ("Floating islands (graph 1)"), filled again by later
conversions and removed when its output goes: columns of ground, air,
then rock, exported to Minecraft as they are.

**Working on a map that exists.** The **Map** node gives the map as it
shows from the sky — its ground with the rock its levels add (towers,
floating islands, overhangs), the pits they carve and what was sculpted in
3D — with where its water lies and the colours of its terrains, read again
whenever the map changes (the levels the graph writes itself are left out).
Wire it where a landform would go: into Rivers, Lakes, Erosion, masks of
slope or height… The template **Paint this map** paints it from its relief
without moving a block of it — the tops of its towers and the floors of its
pits included — and **Rivers on this map** traces rivers in its valleys.

**Rivers.** The Rivers node finds the way the water of the whole relief
goes: flooded up from the sea and the map's edges, every place drains to
its lowest way out, across hollows by their bottom, across flats as water
would wander. Where enough of the land drains (**Fewer**), a river runs: a
main stem from each mouth up to its farthest source, its tributaries off
it. Each is a smooth line laid in the bottom of its valley on the relief
itself, wider and deeper as water joins it (**Width** and **Depth** are the
largest river's); its water only ever goes down, stays under its banks and
meets the sea. Beds and banks (**Banks**: how far they slope down to the
water) are dug at every sample of the build — a 16k build gets rivers drawn
at the block, in under a second at 8k. The hollows a river runs through hold
lakes up to where they spill (**Lakes**: which ones; through the others the
river cuts a gorge). Its outputs: the dug relief, the beds as a mask (gravel
in them), and the water's level for the **Water output**.

Wire the Map's **Water level** into the Rivers' second input (the template
does) and the water already on the map — rivers and lakes painted in
WorldPainter — is kept: the rivers traced keep to its beds and join them,
nothing is dug where it lies, and it stays in the water given back. Rivers
over a coastal plain barely above the sea keep their water down to the
coast.

**Climate.** **Temperature** is warm on one side of the map and cold on the
other, colder with height; **Humidity** comes from the sea, rides the wind
inland and rains out on the slopes facing it, leaving the land behind the
mountains dry. Both are masks (temperature from −30 °C to +45 °C).
**Biomes** gives the colours of the climate they make, and **Biome output**
writes it to the map: every column gets the biome of the game for its
climate (snowy plains, taiga, plains, forest, swamp, savanna, desert,
badlands, jungle, beaches, stony or frozen peaks, the oceans by their
warmth, rivers where the water runs) and, painted, its ground to suit
(snow, grass, sand, red sand, rock, gravel).

**Tectonic plates.** A landform from plates, continental (land) or oceanic
(under the sea), each moving its own way: plates running into each other
raise ranges (the highest between two continents, a coastal range over a
trench where an ocean dives under a continent, an arc of islands between two
oceans), plates pulling apart open rifts or ridges under the sea, plates
sliding past each other leave faults; coasts wind, with capes, bays and
islands. In its settings, the plates are on a map of their own: drag one to
move it, the tip of its arrow to change how it moves, right-click it to make
it a continent or an ocean or to remove it; add plates, or scatter them
anew. The template **Continents** puts it all together.

**Over the map, not into it.** With "over the map" checked (the default
for a graph that starts from the map), converting leaves the map's ground
as it is: what the graph digs goes to a Carve level, what it raises to a
Solid level, with its water, both written again at every conversion.
Change the graph, or paint the map, and convert again: nothing is baked
in. Deleting the two levels gives the map back; unchecked, the graph
replaces the ground.

**Size** says how much of the world the terrain covers: the map as it is
(the default: it writes to the map's own tiles), or a square of 1k to 32k
blocks around its middle — larger than the map, converting adds the tiles
it lacks, so a graph started on a 1k map makes a 16k world when set to 16k.
The viewport says the size it shows. Nodes measure in blocks, so a larger
terrain holds more mountains of the same size, not larger ones (templates
fit their features to it). Converting always works at one sample per block,
up to 16,384 a side, a little coarser beyond.

**Convert to voxels** builds the graph over all of its size on another
thread, showing which node it is at, then writes it to the map as one
undoable step: the heights; the colour output turned
into real blocks of the game (from natural ground, rock, desert, snow and
ice, or every block), each column taking the block whose colour is closest
— by hue before lightness, so that a dark green is moss or grass rather
than mud — two neighbouring blocks mixing where the colour lies between
them; the terrain outputs painted over it; and the layers as levels over
the ground. Distances are in blocks, so a graph looks
the same at every resolution. The graph is saved with the world.

Builds are made for large maps: nodes run on every core, erosion in
parallel blocks; simulations run at 4,096 samples at most and carry what
they changed to the full resolution, so the finest details are kept. The
starter graph builds at 16,384 × 16,384 in about 25 seconds and writes to a
16k map in about 2 more, in about 5.5 GB of memory; 8,192 × 8,192 takes
about 12 seconds and 2 GB. The floating mountains, with their two layers,
take about 55 seconds and 8 GB at 16,384 × 16,384.

### Controls

Massif follows WorldPainter's defaults.

| Input | Action |
|---|---|
| Left / right button (or Alt + left) | Apply the tool / its inverse (lower, erase, drain…) |
| Scroll | Brush size (Shift: one block at a time) |
| ⌘ + scroll (the Windows key on a PC keyboard) | Turn the brush, 5° a notch (Shift: 1°); Ctrl + Shift + scroll on Windows and Linux |
| Ctrl or Alt + scroll, pinch | Zoom the map |
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
| Header buttons | Look at the middle of the map view; show the whole map |
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

Sculpting in 3D has its own editor, **3D Sculpt** (its workspace sits next
to 3D; the 3D View's **Sculpt** button opens it): a 3D view with its tools
along the top — **Sculpt** (the left button adds matter, the right one
digs) and **Smooth** — the brush (any of the library, custom images too),
its radius and strength. Shift + drag moves the view, Alt + drag (or the
middle button) turns it, Alt + wheel sizes the brush. The brush is laid on
the slope under the cursor, read from the matter around smoothed over five
blocks — not on the face of the block hit — so it glides over the steps of
the terrain without shaking, and a custom brush keeps its shape on a
hillside. The ordinary 3D View only shows the world.

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

### The 3D view

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

Every block of a 60,000 × 50,000 map would take some 13 GB of textures.
Past 400 pages (about 1.9 GB), the view keeps a column every 2, 4, 8 or 16
blocks — the coarsest that fits — and still draws the whole map: each of
its tiles holds that many tiles of the map, their water drawn as a surface,
and the rays are walked in its coarser grid. The header says so. A map of
24,000 × 24,000 blocks (35,000 tiles) is shown at a column every 2 blocks
in 672 MB, built in 25 seconds, nearest the camera first.

### Very large maps

Tiles are kept compressed (cold) and decoded when they are needed, so a
map costs its compressed size. Measured on a hilly map of 60,000 × 50,000
blocks (183,379 tiles): generated in 57 seconds on 10 cores, 1.2 GB
compressed and 1.4 GB of memory at most; a brush 400 blocks across applies
in 2 ms; the whole map read by a graph's Map node, at 512 × 512, in 2
seconds. WorldPainter worlds are imported as a stream: each tile is
converted and freed as it is read.

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
  dialogs, the terrain graph and its viewport (a cratered moon, a desert
  canyon, snowy peaks under clouds, every template) and the real blocks to PNG files in that folder, and
  drives the editor with simulated mouse strokes, 3D sculpting, area
  splitting, converting a terrain graph to the map and painting a real
  block. The real
  blocks are only tested when a Minecraft installation is found.
- `MASSIF_WORLD_SAMPLES=<file.world>` also imports a real WorldPainter world
  through the editor, renders it whole in the 3D view and measures what it
  costs.
- `MASSIF_RIVER_WORLD=<world.massif>` (with `MASSIF_SNAPSHOT_DIR`) runs the
  rivers on a real map as a user does — the Rivers on this map template,
  a river laid out from the mountains to the sea, another let go at a source — and renders
  them from above and in 3D.
- `MASSIF_GRAPH_BUILD=<samples>` times a whole build of the starter graph
  (run it with `--release`), and with `MASSIF_GRAPH_APPLY=1` its writing to
  a map of as many blocks; `MASSIF_GRAPH_TEMPLATE=<word>` times a template
  instead (`floating`, `karst`…).

### In the game itself

`tools/minecraft_check.py` checks an export in Minecraft: it exports a
world for a version, starts that version's own server on it without a
window (the game installed on the computer: its client jar, libraries and
Java), and has the game confirm, column by column, what
`massif-cli probe` says was written there — the top block, the water over
it, the biome — then reports what matches, what does not and any error the
game logged while reading the chunks. Running a server means accepting
Mojang's EULA: it only starts one with `--accept-eula`.

```bash
tools/minecraft_check.py --world my.massif --version 1.21.11 --minecraft "<game folder>" --accept-eula
```

## Roadmap

- WorldPainter layers (trees, vegetation, ores, frost, roads…) and their
  import from `.world` files, and layers as brush filters and graph outputs.
- Terrain graphs: GPU evaluation, groups and portals, graphs applied to a part of the map, tiled builds
  beyond 16,384 samples, more simulations (glaciers, sediment layers), the
  render drawn from the full-resolution build, carved layers (caves and
  arches) and water on floating rock (lakes, falls), river courses drawn
  in the graph, the direction of rivers' flow for fluid mods.
- The real blocks' textures in the 3D view, and custom terrains mixing
  real blocks in layers.
- Custom build limits in new worlds and export; trees and plants as a sign
  of forests for the automatic biomes.
- Selections, copy and paste, height-map import.
- Nether and End editing from the editor, customisable key bindings.

## License

Massif is declared GPL-3.0-or-later in `Cargo.toml`, like WorldPainter.
