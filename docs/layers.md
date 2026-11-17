# Layers

Everything painted over the land in Massif is a **layer**: the terrain, the
biome, plants, frost and lava, trees and objects, the tree sizes, and the
annotations. Every layer works the same way:

- pick it in the **Layers** panel, and paint it with any brush — the shapes,
  your own brushes, the gradient brush for terrains — and the brush filters
  (only on grass, a height band, a slope…);
- see it on the map and in the 3D view, or hide it with its **eye**;
- **lock** it, so that no brush paints over it or rubs it out;
- open its **settings**, **duplicate** it, **delete** it, or move it in the
  list;
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
| Tree sizes | Where bushes, small, medium or large trees grow | Size |
| Annotations | Marks in fifteen colours, to plan the map | Notes |

The **Trees** slot is WorldPainter's "base tree" and the **Cover** slot its
"base plant", as the Conquest Reforged team uses them: a block has one of
each. The **Under the trees** slot sits beside the Trees slot: a sub-canopy
layer grows under the canopy of the block's tree layer, in the air its
trees leave, never through a trunk or a crown. Frost and Lava share the
Cover slot with plants: a frozen pond holds no flowers.

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
ground, biome, cover, trees, under the trees; then the masks and the
annotations. Within a slot, the order is yours: drag a layer to move it.
As a block holds one layer of a slot, the order changes the list and the
palettes, not the world.

The **eye** hides a layer on the map and in the 3D view, to see what lies
under it; the export writes it all the same. The **lock** keeps a layer's
blocks: brushes neither paint another layer of its slot over them nor rub
them out, until it is unlocked.

## Settings

A layer's settings open in a dialog of their own (double-click it, or
**Settings…** in its menu): its name and colour, and what its kind needs —
a terrain's blocks and strata, a plant layer's plants and density, a tree
layer's objects, spacing, density, turn and slot (Trees or Under the
trees), Frost's snow and ice. **New layer** asks for a kind first.

## Saving, packs, WorldPainter and export

- The map saves every layer, its settings and what is painted.
- A pack (`.massifpack`) carries terrains, plant layers and tree layers with
  their settings and slot, to share; what is painted stays in the map.
- A WorldPainter `.world` brings its layers in as Massif's: see the table in
  the guide (Layers ▸ WorldPainter's layers in Massif).
- The export writes the ground, then the biome, the cover, the trees and
  then the sub-canopy, each block holding one layer of each slot.

## For developers

- Each slot is a plane of the tile: `terrain`, `biome`, `cover`, `objects`
  (Trees), `understory` (Under the trees), `sizes` (Size) and
  `annotations` (Notes); a plane holds 0 for none, or the layer's number in
  the registry (`TerrainId`, `BiomeId`, `Registry::covers`,
  `Registry::object_layers`). A plane is written only where it holds
  something, so maps without it are unchanged, and a version that does not
  know a plane keeps it as it read it.
- Every slot is painted by `ops::paint_plane`: the brush, its dab, the
  filters and the selection apply to all of them; a locked layer's number
  is skipped.
- `objects::tile_placements` stands the Trees slot's objects first and the
  Under the trees slot's after, with the object of the size the Size plane
  asks for (`ObjectLayerDef::pick`); the export lays the sub-canopy's blocks
  only in air, water and plants.
- Packs carry `CoverDef` (with `frost` and `lava`), `TerrainDef` and
  `ObjectLayerDef` (with `understory`).

## Steps

1. **Slots for trees, sizes, and the rest of WorldPainter's layers** — the
   Trees and Under the trees slots, the Size mask and its tool, the four
   sizes of every kind of tree of the packs; caves, tunnels, custom ground
   covers and combined layers from a `.world`.
2. **One Layers panel** — every layer in one list, by slot, with its eye,
   its lock, its menu (Settings…, Duplicate, Delete) and drag to reorder;
   **New layer** by kind; a settings dialog for each kind instead of
   settings in Tool Settings.
3. **Lock** — brushes skip a locked layer's blocks.
