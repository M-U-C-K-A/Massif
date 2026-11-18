# Village packs

The **village brush** fills a selection with a village: buildings of a
theme, roads that follow the land and join them, and walls if you want
them. It works with no pack at all — its built-in themes are the game's own
villages (plains, desert, savanna, taiga, snowy), read from your game — and
with packs that builders make: folders of schematics and a small manifest.

This page is for the builders who make packs.

## A pack is a folder

```
Medieval Towns/                 the pack
  village.json                  its manifest (optional, see below)
  preview.png                   its picture (optional)
  Countryside/                  a theme
    village.json                the theme's own settings (optional)
    house/                      a type of building
      cottage_1.schem
      cottage_2.schem
      longhouse.nbt
    church/
      chapel.schem
    forge/
      smithy.litematic
    watchtower/
      tower_1.schem
    tavern/
    farm/
    well/
    market/
    walls/                      the pieces of the walls (see below)
      segment.schem
      tower.schem
      gate.schem
  Winter/
    house/
    …
```

Names follow `theme/type/name.schem`. A schematic may be a Sponge
`.schem`, an MCEdit `.schematic`, a structure block's `.nbt`, a Litematica
`.litematic` or an Axiom `.bp`. A type is any folder name: the brush knows
`house`, `church`, `forge`, `watchtower`, `tavern`, `farm`, `well`,
`market`, `stable`, `mill` and `walls`, and takes any other (a `bakery`, a
`guild_hall`) as a type of its own, a house unless the manifest says
otherwise.

## Conventions for the schematics

- **The entrance faces south** (+Z, toward the bottom of the map in
  Massif). The brush turns each building
  so that its entrance faces the road it stands on. Put the door — or the
  gap that serves as one — on the south wall, at floor level.
- **Foundations: 3 to 4 layers** under the floor, of stone, cobblestone or
  the like. The brush sinks them into the ground and levels the ground
  around the building to its floor: on a slope they show as a terrace wall.
- **Structure voids around** the building, not air: the land shows through
  them. Air carves: use it inside the rooms only.
- **No loose entities**: villagers and animals belong in the manifest
  (`"villagers"`), not in the schematic.
- Chests may keep their contents, or name a loot table (the game fills
  them when first opened). Signs, banners and item frames are kept.
- Keep each building within 48 × 48 blocks; a market or a castle may be
  larger, and is then placed where the land has room for it.

## The manifest

`village.json`, in the pack's folder (for all its themes) or in a theme's
folder (for that theme), says what the brush cannot guess. Every field is
optional.

```json
{
  "name": "Countryside",
  "author": "A builder",
  "description": "Half-timbered cottages, a chapel, a smithy.",
  "roads": {
    "main": { "width": 3, "blocks": { "minecraft:dirt_path": 4, "minecraft:coarse_dirt": 1 } },
    "side": { "width": 2, "blocks": { "minecraft:dirt_path": 3, "minecraft:grass_block": 1 } }
  },
  "types": {
    "house":      { "share": 0.55 },
    "farm":       { "share": 0.15, "edge": true },
    "church":     { "share": 0.04, "max": 1, "centre": true },
    "well":       { "share": 0.03, "max": 1, "centre": true },
    "forge":      { "share": 0.05, "max": 2 },
    "tavern":     { "share": 0.04, "max": 1, "near": "market" },
    "watchtower": { "share": 0.03, "edge": true },
    "market":     { "share": 0.04, "max": 1, "centre": true }
  },
  "foundation": 3,
  "villagers": 1,
  "walls": { "height": 6, "block": "minecraft:stone_bricks", "towers": 12 }
}
```

| Field | What it says |
|---|---|
| `roads.main`, `roads.side` | The blocks of the main roads and of the side roads, with their weights, and their width. Default: the game's dirt path, 3 and 2 wide. |
| `types.<type>.share` | Its share of the buildings: 0.55 makes about 55 % of them houses. Types left out share what is left equally. |
| `types.<type>.max` | At most this many in a district (a church, a well). |
| `types.<type>.centre` | Placed near the middle of its district. |
| `types.<type>.edge` | Placed near the edge of the village (farms, watchtowers). |
| `types.<type>.near` | Placed near a building of that type. |
| `foundation` | The layers under the floor, if not 3. |
| `villagers` | Villagers per house (0 for none). |
| `walls` | Walls without pieces of their own: their height, their block, a tower every so many blocks. |

## Walls

With **Walls** ticked, the brush runs a wall round the village, inside the
selection's edge: corner towers where it turns, a tower every `towers`
blocks along it, a gate wherever a road crosses it. It follows the land.

The pieces come from the theme's `walls/` folder when it has them:

- `segment.schem` — a straight piece of wall, along x, its outside to the
  south, as long as you like (16 is a good length): repeated end to end.
- `tower.schem` — a tower, its middle on the wall's line.
- `gate.schem` — a gate, the road passing through it north to south.

Without them, walls of the manifest's block (or stone bricks), crenellated,
with towers of the same.

## Sizes

The brush sizes the village by the selection: about 100 × 100 blocks gives a
hamlet of a handful of buildings round a well; 300 × 300, a village round
its square; 1000 × 1000, a town of several districts, each round its own
square, joined by main roads.
