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
| Annotations | Marks in fifteen colours, to plan the map | Notes |

The **Trees** slot is WorldPainter's "base tree" and the **Cover** slot its
"base plant", as the Conquest Reforged team uses them: a block has one of
each. The **Under the trees** slot sits beside the Trees slot: a sub-canopy
layer grows under the canopy of the block's tree layer, in the air its
trees leave, never through a trunk or a crown. Frost and Lava share the
Cover slot with plants: a frozen pond holds no flowers. The
**Underground** slot lies in the rock, under the strata of the ground's
terrain: a pocket layer never meets the ground's blocks, its cover or its
trees.

Water itself is not a layer: Flood, the River tool and the Water brush
shape it. Levels and 3D sculpting are not layers either: they shape
the land the layers lie on.

## Masks

A **mask** is a layer that other layers read. **Tree sizes** marks where
bushes, small, medium and large trees grow; a tree layer painted over it
takes its objects of the size marked: paint the sizes first, then one
species brush over everything. A layer's objects are split by their height
into the four sizes, a quarter of them each; every kind of tree of the packs
comes in the four sizes. Masks are never exported, and neither are
annotations.

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

## Settings

A ground cover's or a tree layer's settings open in a dialog of their own
(**Settings…** in its menu), and in Tool Settings while it is painted: its
name and colour (a ground cover's with the opacity it shows at on the map),
and what its kind needs — a plant layer's plants, density
and what it does to the water (Frost, Lava), a tree layer's objects and
their sizes, spacing, density, turn and slot (Base or Sub-canopy), a
pocket layer's blocks, share of the rock, size and heights, a combined
layer's layer of each slot and its share. A double-click renames a layer.
Each section has its button to add one: New combined, New mix, New
cover, Import objects…, New pockets.

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
| Custom object layer (Bo2, schematics, structures) | An object layer of its objects, read from the file, with their offset, frequency, random turn, density and grid; the trees of WorldPainter's tree pack, the pack's own |
| Ground cover layer | Laid over the land: a ground cover of its blocks, as high as it is thick. Taking the place of the top of the land: a terrain of its blocks, that deep (in a `.world`, a ground cover on the ground lowered as deep) |
| Underground pockets layer | Underground pockets of the same blocks, as frequent and as large, between the same heights |
| Combined layer | A combined layer: its terrain (a custom one from the material written with it), its biome, and the first layer of each slot it holds at its factor; the others it holds are layers of their own |
| Tunnel and floating layers | Not carried over: Massif carves and raises land with Carve and Solid levels |
| Custom annotation layers | Not carried over: Massif's annotations are fifteen colours |

Not carried over either, and said in the report: the collision,
underground, leaf decay, foundation and height settings of a custom object
layer's objects, and their entities (an object of air or entities only is
empty); a ground cover's edge shape and noise; farmland under crops. A
layer of the map is never changed: one with the name of a layer of the file
but other settings stays as it is, the file's becoming a layer of its own
("Meadow 2"); a plant layer imported twice is one ground cover.

## For developers

- Each slot is a plane of the tile: `terrain`, `biome`, `cover`, `objects`
  (Trees), `understory` (Under the trees), `pockets` (Underground), `sizes`
  (Size) and `annotations` (Notes); a plane holds 0 for none, or the
  layer's number in the registry (`TerrainId`, `BiomeId`,
  `Registry::covers`, `Registry::object_layers`, `Registry::pockets`). A plane is written only where it holds
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
