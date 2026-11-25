# Layers

Everything painted over the land in Massif is a **layer**: the terrain, the
biome, plants, frost and lava, trees and objects, the tree sizes, the
pockets in the rock, and the annotations. Every layer works the same way:

- pick it in the **Layers** panel, and paint it with any brush — the shapes,
  your own brushes, the gradient brush for terrains — and the brush filters
  (only on grass, a height band, a slope…);
- see it on the map and in the 3D view, or hide it with its **eye**;
- **lock** it, so that no brush paints over its blocks or rubs it out;
- open its **settings** in a dialog, **duplicate** it, **delete** it, or
  move it up and down its section;
- save it with the map, share it in a pack, bring it from WorldPainter,
  and export it.

A **combined layer** paints several of them at once (see below).

## Kinds and slots

A block holds **one layer of each slot**. Painting another layer of the
same slot over a block takes the place of the one there, so two terrains,
two plants or two tree species never pile up on a block.

| Kind | What it paints | Slot |
|---|---|---|
| Terrain | The blocks of the ground: one terrain, a mix by noise, a gradient | Ground |
| Biome | The biome | Biome |
| Plants | Grass, flowers, any blocks growing on the ground | Cover |
| Frost, Lava | Snow on the land and ice on the water; water turned to lava | Cover |
| Trees and objects | Trees, rocks, ruins, from schematics or the packs | Trees |
| Sub-canopy | Bushes, hazel, holly growing under the trees | Under the trees |
| Underground pockets | Gravel, clay, a mod's stone: blobs of other blocks in the rock | Underground |
| Tree sizes | Where bushes, small, medium or large trees grow | Size |
| Annotations | Marks in colour (WorldPainter's fifteen and the map's own), painted or drawn as curves, to plan the map | Notes |

The **Trees** slot is WorldPainter's "base tree" and the **Cover** slot its
"base plant", as the Conquest Reforged team uses them: a block has one of
each. The **Under the trees** slot sits beside the Trees slot: a sub-canopy
layer grows under the canopy of the block's tree layer, in the air its
trees leave, never through a trunk or a crown. Frost and Lava share the
Cover slot with plants: a frozen pond holds no flowers. The
**Underground** slot lies in the rock, under the strata of the ground's
terrain: a pocket layer never meets the ground's blocks, its cover or its
trees.

A map brought from WorldPainter keeps what WorldPainter allows instead:
**several layers of a slot on a block**, each with its own intensity
(three ground covers, a forest at 50 % and bushes at 15 %). Its covers
lie over each other as WorldPainter lays them — custom ground covers on
the ground, plants on them, frost and lava over everything — and the
export grows each in turn; each of its tree layers stands its trees
thinned to its intensity there. The Info panel lists them all. Rubbing one
out with the right button takes it alone out (the cover under it shows
again); painting a layer of the slot over the block takes the place of
all of them, as a slot holds one. A map without such blocks is saved as
before.

Water itself is not a layer: Flood, the River tool and the Water brush
shape it. Levels and 3D sculpting are not layers either: they shape
the land the layers lie on.

## Masks

A **mask** is a layer that other layers read. **Tree sizes** marks where
bushes, small, medium and large trees grow — or sizes of your own; a tree
layer painted over it takes its objects of the size marked: paint the
sizes first, then one species brush over everything. Masks are never
exported, and neither are annotations.

### Tree sizes

Every map has four sizes, Bushes, Small, Medium and Large trees; the Tree
sizes tool's settings add sizes of your own (Tiny, Forest, Open-grown…),
rename them, recolour them, and remove one's own (where it was painted,
any size stands). Each object of a layer has its size: set by hand in the
layer's object list (**Tree size**), or **Auto**, guessed from its height
among the layer's other objects left on Auto (the lowest quarter bushes,
then small, medium and large trees). Where a layer has no object of the
size marked, the nearest size it has stands.

A layer can be spaced **by canopy** (new layers of imported objects
are) or **on a grid**. On a grid, at most one object stands in each
square of the layer's spacing, whatever its size, as WorldPainter places
them: a spacing wide enough for broad crowns leaves narrow trees far
apart. **By canopy**, each object has the radius of its crown: measured
from its leaves (how far from the middle of the leaves the columns
holding leaves reach, the farthest tenth left out, so a stray branch or
the trunk does not count; an object without leaves is measured by all its
blocks), or set by hand in the object list (**Canopy**). Two neighbours
stand at least as far apart as their radii added, so that at full density
their crowns touch without overlapping, narrow and wide trees mixed, and
never closer than the layer's **Least gap** between trunks. The wider
crowns are placed first and the narrow ones fill what they leave, as
under a real canopy. Measured on a forest of narrow (radius 1–2) and wide
(5–6) trees at full density: 61 % of the land under a crown and no
clearing wider than 4.4 blocks, against 32 % and 10.2 blocks on a grid
of 14. The same holds across the edges of tiles, by what is painted on
the ground there.

Each layer has a **density of each size**, a share of its density: 40 %
bushes, 100 % large trees… By canopy, the density thins the trees that
stand, leaving clearings rather than room for others. Crowns are circles
for now; irregular shapes may come later.

**Edit ▸ Import mask…** paints any of these from an image, as WorldPainter's
Import mask: the annotations (each pixel's nearest of the 15 colours, or one
colour where the image is on), a layer of any slot (a ground cover, trees,
under the trees, pockets, a tree size), a terrain, a biome, water or the
height. A grey pixel is a strength: on from a threshold, or a share of the
columns painted, scattered. With **Replace**, **Add**, **Max** and **Min**
it goes with what is painted; locked layers keep their columns. **File ▸
Export Region…** writes each terrain and layer of a part of the map as such
an image, to work on elsewhere and import back in place. See Import mask in
the guide.

## Combined layers

A **combined layer** is a brush of several layers, as WorldPainter's
combined layers: one layer of each slot it names — a terrain, a biome, a
ground cover, a tree layer, a sub-canopy layer, a tree size, a pocket
layer — each over its **share** of the columns painted. It has no slot
and no blocks of its own: one stroke writes each of its layers into its
slot, as that layer's own tool does (locks included), one step to undo;
so a block still holds one layer of each slot, and changing a combined
layer changes what it paints next, not what it painted. A share is drawn
once for good: painting again over the same blocks paints the same
columns. The right button rubs out what it paints: the slots it names
emptied, the biome back to automatic, and the terrain put back as Paint
terrain's right button puts it. Its pockets go into the rock of the
ground, whatever level is edited. A WorldPainter combined layer comes in
as one, its layers at their factors.

### Later: objects one block up

Conquest Reforged's trees are made to stand one block over the ground, so
that a layer (moss, roots, a ground cover of its own) can be painted under
them. An object's own rules can raise it already (**Raised by** 1); a
layer-wide setting, or a Conquest preset that raises its trees and lets a
cover grow under them, is left for later.

## Order, visibility and lock

The panel lists the combined layers first, then the layers by slot, in
the order the export lays them: terrain mixes (the ground), ground
covers, trees and objects, under the trees, underground pockets; then
the tree sizes and the annotations. Biomes are painted from
the Biome panel. Within a section, the order is yours: **Move up** and
**Move down** in a layer's menu. As a block holds one layer of a slot, the
order changes the list, not the world.

The **eye** hides a layer on the map and in the 3D view, to see what lies
under it; the export writes it all the same. The **lock** (pointed at, a
row shows it on its right; also in its menu) keeps a ground cover's, a
tree layer's or a pocket layer's blocks: brushes neither paint another layer of its slot over
them nor rub them out, until it is unlocked. Terrains are kept the way
WorldPainter keeps them, by the brush filters (**Except on** a terrain).

## Palettes

**Palettes** sort the layers as WorldPainter's palettes do: named groups
of layers of any kind, shown at the top of the Layers panel, each folding
under its title. **New palette** makes one; a layer joins one when it is
dragged onto it, or by its menu (▸ **Palette**), and is then listed in it
rather than in its section, with what it is and does unchanged: its slot,
its order in the export, its lock and eye. Dragged onto a section's title,
or by ▸ Palette ▸ **Back to its section**, it goes back. A palette's menu
renames it, moves it up or down, or deletes it: its layers go back to
their sections, none is removed. Palettes are a way of listing layers, not
of painting them: they never change the map's blocks.

A palette's **eye**, on the left of its title, hides or shows every layer
of it at once on the map and in the 3D views, ground covers and object
layers alike, as each layer's own eye does (the export is never changed).
When some of its layers are hidden and some shown, it is half an eye, and
a click shows them all. **Alt-click** the eye, or ▸ **Show only this
palette** in its menu, to show that palette alone, as WorldPainter's
solo: every other cover and object layer is hidden; again, the layers are
shown as they were.

A single layer has its **solo** too: **Alt-click** its eye, or ▸ **Solo**
in its menu, and it is the only cover or object layer drawn on the map and
in the 3D views (the terrain is still drawn); a combined layer shows the
cover and trees it paints. Its eye turns to the accent, and the map's
header shows **Solo: its name**. Alt-click the eye again, ▸ **Show all
layers**, or the chip in the map's header, and every layer is shown or
hidden exactly as before the solo. The map keeps a solo when it is saved,
with the visibility to come back to.

The map saves its palettes and which layer is in which; a terrain mix,
kept with the app's settings rather than the map, keeps its palette too.
A WorldPainter world or `.layer` file brings each custom layer in the
palette WorldPainter sorted it into, in its place there; a palette
hidden in WorldPainter stays hidden, and a soloed one stays alone.

Your own **brushes** have palettes of their own: the groups of the
Brushes panel, each a folder of Massif's brushes folder. They fold,
rename and delete (their brushes go to Custom Brushes), and a brush moves
to another one when it is dragged there or by its menu.

## Settings

A ground cover's or a tree layer's settings open in a dialog of their own
(**Settings…** in its menu), and in Tool Settings while it is painted: its
name and colour (a ground cover's with the opacity it shows at on the map),
and what its kind needs — a plant layer's plants, density
and what it does to the water (Frost, Lava), a tree layer's objects and
their sizes and canopies, spacing (by canopy or on a grid), density and density of each size, turn and slot (Base or Sub-canopy), a
pocket layer's blocks, share of the rock, size and heights, a combined
layer's layer of each slot and its share. A double-click renames a layer.
Each section has its button to add one: New combined, New mix, New
cover, Import objects…, New pockets.

## Object settings

A tree or object layer lists its objects one a row, as WorldPainter's
object layer dialog: a click picks one, **Shift+Click** or
**Shift+↑/↓** a run of them, **⌘/Ctrl+Click** adds or takes one out,
**⌘/Ctrl+A** all of them. The settings under the list are those of the
objects picked: a change goes to each, and a setting they do not share
reads *mixed*. An object two layers share is copied before one of them
changes it, so that the other keeps it as it was.

A tree or object layer's own window (**Settings…** in its menu) is wide,
for layers of hundreds of schematics: on the left, the tall list of its
objects, with a search (by name or file) and a sort (**Imported**,
**Name**, **Height**, **Frequency**), each row with its picture, its
measures, its tree size and its share of the layer; **Add objects…** over
it. On the right, the layer's settings, and under them those of the
objects picked, with **Take out of the layer**. Picking works as above,
along the order shown. Each pane scrolls on its own; drag the window's
corner to make it bigger or smaller, and it opens again at that size.

Sizes of one's own are made where they are used: **New size…** at the end
of an object's **Tree size** list adds one and gives it to the objects
picked, and **New size** under **Density of each size** adds one to the
map. Rename them, and paint them, with the Tree sizes tool.

Each object has its **frequency** against the others and its **origin**,
the block of it set on the spot over the ground. With **Own rules**, it
has every attribute of WorldPainter's object dialog, in place of its
layer's turning, mirroring, Under water and Replace the ground (the
layer's Only on soil and Sink still hold):

| Setting | What it does |
|---|---|
| Turned, Mirrored | At random, or always as set |
| Height, Raised by, Varied by | Over the ground or at a fixed height; that many blocks higher; up to that many higher or lower, at random |
| On land, Under water, On water, Under lava, On lava | Where it may stand: dry ground, the bottom or the top of water and lava |
| On air | At a fixed height over the ground, it floats rather than being left out |
| Collides with the floor | Standing on water, its blocks under the water stop at the floor |
| Extends its foundation | Its lowest blocks go on down to the ground where it would float |
| Collides with | Over the ground, it takes the place of air only (All), of air, water and plants (Solid), or of anything (None) |
| Replaces underground | All its blocks in the ground replace it, its solid ones, or none (they only fill air) |
| Leaves | As they are, made to decay, or never to |
| Waterlogged | By the water it stands in, or as the object has them |
| Joins fences and panes | Its fences, panes, bars and walls join their neighbours in it |
| Placed as air | A block of it written as air: a hollow in the ground |

The map and the 3D view show where each object stands, turned and raised
as its rules say; its blocks' rules (collisions, leaves, joins, air) are
the export's. Objects without own rules export byte for byte as before.

## Saving, packs, WorldPainter and export

- The map saves every layer, its settings and what is painted, and its
  combined layers.
- A pack (`.massifpack`) carries terrains, plant layers and tree layers with
  their settings and slot, to share; what is painted stays in the map.
- A WorldPainter `.world` brings its layers in as Massif's: see the table in
  the guide (Layers ▸ WorldPainter's layers in Massif).
- A WorldPainter `.layer` (a custom layer exported from its menu) comes in
  as Massif's layers too, experimentally: see below.
- The export writes the ground, then the biome, the pockets in the rock
  under the ground's strata, the cover, the trees and then the sub-canopy,
  each block holding one layer of each slot.

## WorldPainter layer files (experimental)

**Import WorldPainter layer…**, at the top of the Layers panel, reads
`.layer` files, several at once; dropping them on the window does the same.
Each becomes layers of the map, by the same mapping as a `.world`'s, and the
first one made is picked to paint with. A report says what each became and
what was not carried over.

| WorldPainter `.layer` | In Massif |
|---|---|
| Plant layer | A ground cover of the same plants, in the same shares, on valid blocks only if it asked |
| Custom object layer (Bo2, schematics, structures) | An object layer of its objects, read from the file, with their offset, frequency, density and grid, and every attribute of each object as its own rules (see Object settings); the trees of WorldPainter's tree pack, the pack's own |
| Ground cover layer | Laid over the land: a ground cover of its blocks, as high as it is thick. Taking the place of the top of the land: a terrain of its blocks, that deep (in a `.world`, a ground cover on the ground lowered as deep) |
| Underground pockets layer | Underground pockets of the same blocks, as frequent and as large, between the same heights |
| Combined layer | A combined layer: its terrain (a custom one from the material written with it), its biome, and the first layer of each slot it holds at its factor; the others it holds are layers of their own |
| Tunnel and floating layers | Not carried over: Massif carves and raises land with Carve and Solid levels |
| Custom annotation layers | Not carried over: Massif's annotations are fifteen colours |

Not carried over either, and said in the report: the entities of a
custom object layer's objects (an object of air or entities only is
empty) and the old form of their Replace with air; a ground cover's edge
shape and noise; farmland under crops. A
layer of the map is never changed: one with the name of a layer of the file
but other settings stays as it is, the file's becoming a layer of its own
("Meadow 2"); a plant layer imported twice is one ground cover.

## For developers

- Each slot is a plane of the tile: `terrain`, `biome`, `cover`, `objects`
  (Trees), `understory` (Under the trees), `pockets` (Underground), `sizes`
  (Size) and `annotations` (Notes); a plane holds 0 for none, or the
  layer's number in the registry (`TerrainId`, `BiomeId`,
  `Registry::covers`, `Registry::object_layers`, `Registry::pockets`; for
  annotations 1 to 15, WorldPainter's colours, then the map's own in
  `Registry::annotations`, up to 255, each with its opacity). A plane is written only where it holds
  something, so maps without it are unchanged, and a version that does not
  know a plane keeps it as it read it.
- Every slot is painted by `ops::paint_plane`: the brush, its dab, the
  filters and the selection apply to all of them. `History::locks`
  (`registry::Locks::of`, set by the tool before it paints) holds the
  locked covers, object layers and pocket layers; `paint_plane` skips a column whose
  layer in that plane is locked, painting or rubbing out.
- `order` (covers, object layers, pocket layers) is a layer's place in its section of the
  panel; `locked` its lock; `hidden` its eye. None of them changes the
  export.
- A pocket layer's blocks are the registry's (`BlockId`), as a terrain's.
  `pockets::Pockets` says which block of a column is a pocket's, the same
  for the export, the survival check and the cut of the Ores window
  (`ores::rock_at` takes the column's pockets), so that they agree block
  for block, ores included.
- `objects::tile_placements` stands the Trees slot's objects first and the
  Under the trees slot's after, with the object of the size the Size plane
  asks for (`ObjectLayerDef::pick`); the export lays the sub-canopy's blocks
  only in air, water and plants.
- Packs carry `CoverDef` (with `frost` and `lava`), `TerrainDef` and
  `ObjectLayerDef` (with `understory`).
- `massif_legacy::layer_file::read_layer_file` reads a `.layer` (gzip, then
  a Java stream of one `CustomLayer`) with the `.world` import's
  `Converter` into a copy of the map's registry, restores any layer of the
  map the conversion replaced by name, and returns what was made
  (`Made`) and left out (`LeftOut`). Samples: `crates/massif-legacy/tests/data/layers`,
  written by `tools/wp-layers/MakeLayers.java` with WorldPainter's classes;
  `MASSIF_WP_LAYERS=<folder>` reads more (third-party objects).
- A combined layer (`combined::CombinedDef`, in `Registry::combined`)
  holds `Component`s, one of each `Part`, with the layer's number in its
  slot and its share. `CombinedDef::to_paint` reads them against the
  registry (a removed layer left out, a tree layer in the slot it is in
  now) and `ops::paint_parts` paints them through `paint_plane`, the
  share a mask of the columns drawn from a seed of the part.

## Steps

1. **Slots for trees, sizes, and the rest of WorldPainter's layers**
   (landed) — the Trees and Under the trees slots, the Size mask and its
   tool, the four sizes of every kind of tree of the packs; caves, tunnels,
   custom ground covers and combined layers from a `.world`.
2. **One Layers panel and the lock** — the sections by slot (terrain
   mixes, ground covers, trees and objects, under the trees, tree sizes,
   annotations), each row with its eye and its lock, a menu with
   Settings…, Paint, Rename, Duplicate, Move up and down, Lock, Hide and
   Remove, and a dialog with the settings of a cover or a tree layer;
   brushes skip a locked layer's blocks (landed).
3. **Underground pockets** — the Underground slot and its tool, pocket
   layers of any blocks with their share of the rock, size and heights
   (`PocketDef::block_at`, the same blocks for the export and every count),
   and WorldPainter's underground pockets from a `.world`.
4. **Combined layers** — one layer that paints several at once (a
   terrain, a biome, a cover, a tree layer, pockets), each over its own
   share, with an editor of its own; WorldPainter's combined layers
   become one.
