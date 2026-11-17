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

Water itself is not a layer: Flood, Drain, the River tool and the Water
brush shape it. Levels and 3D sculpting are not layers either: they shape
the land the layers lie on.

## Masks

A **mask** is a layer that other layers read. **Tree sizes** marks where
bushes, small, medium and large trees grow; a tree layer painted over it
takes its objects of the size marked: paint the sizes first, then one
species brush over everything. A layer's objects are split by their height
into the four sizes, a quarter of them each; every kind of tree of the packs
comes in the four sizes. Masks are never exported, and neither are
annotations.

## Order, visibility and lock

The panel lists the layers by slot, in the order the export lays them:
terrain mixes (the ground), ground covers, trees and objects, under the
trees, underground pockets; then the tree sizes and the annotations. Biomes are painted from
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
name and colour, and what its kind needs — a plant layer's plants, density
and what it does to the water (Frost, Lava), a tree layer's objects and
their sizes, spacing, density, turn and slot (Base or Sub-canopy), a
pocket layer's blocks, share of the rock, size and heights. A
double-click renames a layer. Each section has its button to add one: New
mix, New cover, Import objects…, New pockets.

## Saving, packs, WorldPainter and export

- The map saves every layer, its settings and what is painted.
- A pack (`.massifpack`) carries terrains, plant layers and tree layers with
  their settings and slot, to share; what is painted stays in the map.
- A WorldPainter `.world` brings its layers in as Massif's: see the table in
  the guide (Layers ▸ WorldPainter's layers in Massif).
- The export writes the ground, then the biome, the pockets in the rock
  under the ground's strata, the cover, the trees and then the sub-canopy,
  each block holding one layer of each slot.

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
   terrain, a biome, a cover, a tree layer, pockets), each at its own
   strength, with an editor of its own; WorldPainter's combined layers
   become one.
