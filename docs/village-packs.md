# Village packs

The **village brush** fills a selection with a village: buildings of a
theme, roads that follow the land and join them, props along the roads,
fields by the farms, and walls if you want them. It works with no pack at
all — its built-in themes are the game's own villages (plains, desert,
savanna, taiga, snowy), read from your game — and with packs that builders
make: folders of schematics and a small manifest.

This page is for the builders who make packs.

## A pack is a folder

```
Plains/                         the pack (here a pack of one theme)
  village.json                  its manifest (optional, see below)
  house/                        a type of building
    small_1.schem … small_8.schem
    medium_1.schem … medium_6.schem
  manor/                        large houses, farmhouses, manors
    farmhouse_1.schem
    manor_1.schem
  church/
    chapel.schem
    church.schem
  forge/
    blacksmith_1.schem
  tavern/
    inn_1.schem
  watchtower/
    tower_1.schem
  farm/                         barns: each may keep a field beside it
    barn_1.schem … barn_4.schem
  mill/
    windmill.schem
    watermill.schem             stands by water (see "Mills")
  well/
    well_1.schem
  market/                       stalls, set round the square
    stall_1.schem … stall_4.schem
  props/                        small things along the roads and by buildings
    lamppost_1.schem
    bench_1.schem
    hay_bales_1.schem
    cart_1.schem
  walls/                        the pieces of the walls (optional)
    tower.schem
    gate.schem
```

A pack may also hold several themes, each a folder of type folders
(`Medieval Towns/Countryside/house/…`, `Medieval Towns/Winter/house/…`),
the pack's `village.json` applying to all of them and a theme's own to that
theme.

A schematic may be a Sponge `.schem`, an MCEdit `.schematic`, a structure
block's `.nbt`, a Litematica `.litematic` or an Axiom `.bp`. A type is any
folder name: the brush knows `house`, `manor`, `church`, `forge`,
`watchtower`, `tavern`, `farm`, `stable`, `mill`, `well`, `market`,
`props` and `walls`, and takes any other (a `bakery`, a `guild_hall`) as a
type of its own, placed like a house unless the manifest says otherwise.
Any number of variants per type works; one is enough to start.

## Sizes

The size of a building is read from its footprint, its larger side:

| Class | Larger side | Usual types |
|---|---|---|
| small | up to 11 blocks | small houses |
| medium | 12 to 16 | medium houses, taverns, forges |
| large | 17 to 24 | manors, farmhouses, barns |
| huge | over 24 | a church up to 24 × 40, a castle |

The brush's **Houses** setting (smaller ↔ larger) and the Village
Generator's *Small ↔ large* move the village between these classes: toward
small it draws mostly small houses, toward large more medium and large ones
and more manors. Plots far from the square lean a little larger. The
manifest's `sizes` changes the limits.

The **pack check** compares each schematic with the sizes the list of
Plains builds gives each type (a watchtower 5 × 5 to 9 × 9, a well or a
stall 3 × 3 to 5 × 5, a tavern 12 × 12 to 18 × 18…) and says so when one is
outside them; the building is used as it is all the same.

## Conventions for the schematics

- **The entrance faces south** (+Z, toward the bottom of the map in
  Massif). The brush turns each building so that its entrance faces the
  road it stands on. Put the door — a door's lower half, or a fence gate —
  on the south wall, at floor level. A building whose entrance is on
  another side says so in the manifest (`"entrance": "east"`, for its type
  or for one file).
- **Foundations: 3 to 4 layers** under the floor, of stone, cobblestone or
  the like. The door stands on the floor, so the layers under the door are
  the foundation: the brush sinks them into the ground and levels the
  ground around the building to its floor; on a slope they show as a
  terrace wall. Props stand on the ground and have none.
- **Structure voids around** the building, not air: the land shows through
  them. Air carves: use it inside the rooms only.
- **No loose entities**: villagers and animals belong in the manifest
  (`"villagers"`), not in the schematic.
- Chests may keep their contents, or name a loot table (the game fills
  them when first opened). Signs, banners and item frames are kept.
- Keep each building within 48 × 48 blocks; a castle may be larger, and is
  then placed where the land has room for it.

## The manifest

`village.json`, in the pack's folder (for all its themes) or in a theme's
folder (for that theme), says what the brush cannot guess. Every field is
optional.

```json
{
  "name": "Plains",
  "author": "A builder",
  "description": "Half-timbered cottages, a chapel, a smithy.",
  "sizes": { "small": [7, 11], "medium": [11, 16], "large": [16, 24] },
  "types": {
    "house":      { "share": 0.55 },
    "manor":      { "share": 0.05, "max": 2, "prefer": "high_ground" },
    "church":     { "share": 0.04, "max": 1, "centre": true },
    "forge":      { "share": 0.05, "max": 2, "near": "market" },
    "tavern":     { "share": 0.04, "max": 1, "near": "market",
                    "files": { "inn_2.schem": { "entrance": "east", "weight": 2 } } },
    "watchtower": { "share": 0.03, "max": 2, "edge": true },
    "farm":       { "share": 0.12, "edge": true,
                    "field": { "size": [12, 24], "crops": ["wheat", "carrots", "potatoes"] } },
    "mill":       { "max": 1, "edge": true, "files": { "watermill.schem": { "water": true } } },
    "well":       { "max": 2, "centre": true },
    "market":     { "max": 4 }
  },
  "props": {
    "along_roads": ["lamppost", "bench"],
    "near": { "farm": ["hay_bales", "cart"], "market": ["cart"] }
  },
  "roads": {
    "main": { "width": 3, "blocks": { "minecraft:dirt_path": 4, "minecraft:coarse_dirt": 1 } },
    "side": { "width": 2, "blocks": { "minecraft:dirt_path": 3, "minecraft:grass_block": 1 },
              "edges": { "minecraft:coarse_dirt": 1 } }
  },
  "foundation": 3,
  "villagers": 1,
  "walls": { "height": 6, "block": "minecraft:stone_bricks", "towers": 40 }
}
```

| Field | What it says |
|---|---|
| `sizes` | The limits of the size classes (above). |
| `types.<type>.share` | Its share of the buildings: 0.55 makes about 55 % of them houses. |
| `types.<type>.max` | At most this many in a district (a church, a well). |
| `types.<type>.centre` | Placed near the middle of its district. |
| `types.<type>.edge` | Placed toward the edge of the village (farms, watchtowers, mills). |
| `types.<type>.near` | Placed near a building of that type (a forge near the market). Ignored when the pack has no such type. |
| `types.<type>.prefer` | `"high_ground"`: on the highest plots (a manor). |
| `types.<type>.field` | Each keeps a field behind it, away from the road: its side from and to, in blocks, and the crops drawn from (`wheat`, `carrots`, `potatoes`, `beetroots`). |
| `types.<type>.entrance` | The side its schematics' entrance is on, if not south. |
| `types.<type>.files.<file>` | One schematic's own `entrance`, `weight` (how often it comes up, 1 by default) and, for a mill, `water`. |
| `props.along_roads` | The props set along the roads, the first the most often (lampposts). |
| `props.near` | The props set beside the buildings of a type. |
| `roads.main`, `roads.side` | The blocks of the main and side roads with their weights, their `edges`' blocks if they differ, and their width. Default: the game's dirt path, 3 and 2 wide. |
| `foundation` | The layers under the floor of a building with no door found, if not 3. |
| `villagers` | Villagers per house (0 for none). |
| `walls` | Walls without pieces of their own: their height, their block, a tower every so many blocks. |

## Markets, props and fields

- **Market stalls** (`market/`) stand round each district's square, facing
  it, up to the type's `max` (4 by default). Give `market` a `share` to
  have them placed along the streets like houses instead.
- **Props** (`props/`) are named by their file, the number after the last
  `_` dropped: `lamppost_1` and `lamppost_2` are both lampposts. Without a
  prop of a name, Massif builds a simple one of its own (a lamppost of
  fence and lantern, a bench of stairs, hay bales, a cart). The **Props**
  setting sets how many: none, a few, many.
- **Fields** are farmland with a row of water every nine, planted, and
  fenced (when props are on) with a gate toward the farm.

## Mills

A mill (`mill/`) stands toward the edge of the village, one at most. A
schematic with `water` (or with *water* in its name: `watermill.schem`)
stands within ten blocks of water; the others are windmills. The Village
Generator's *Mill* setting picks one: auto (a watermill when there is water
near, else a windmill), wind, water, or none.

## Walls

With **Walls** set to **Ramparts**, the village gets a wall round it,
inside its edge: corner towers where it turns, a tower every `towers` blocks
along it, a gate wherever a road crosses it. It follows the land. The other
styles (fences, hedges, low walls, a palisade) are Massif's own, of the
game's blocks, and take nothing from the pack.

The wall itself is of the manifest's block (stone bricks unless it says
otherwise), crenellated. Its towers and gates come from the theme's
`walls/` folder when it has them:

- `tower.schem` — a tower, its middle on the wall's line.
- `gate.schem` — a gate, the road passing through it north to south.

Without them, towers of the wall's block, five blocks across, with a floor
on top and a ladder up, and gates that are an arch over the road.

## Roads

The brush's **Roads** setting lays the pack's own road blocks (*Theme's*),
or one of four styles: dirt path, gravel (dirt path and gravel, coarse dirt
on the edges), stone (cobblestone, andesite, stone bricks, slabs where the
road climbs a block) or planks. Over narrow water, roads cross on planks.

## Checking a pack

When a pack is added to the brush, Massif reads it and lists what is wrong
with it under **notes on the packs**, file by file:

- a building with no door on its entrance side (its entrance is then put
  in the middle of that side);
- a door on the lowest layer: no foundation;
- a building far larger or smaller than its type usually is;
- entities saved in a schematic;
- air round a building where structure voids belong;
- the same build saved twice;
- a type the manifest names with no folder;
- a file or manifest that cannot be read (left out).

`massif-cli village-check <pack>` prints the same, with what Massif read
of each type (how many variants of each size). `massif-cli
village-test-pack <dir>` writes a Plains pack of plain boxes at the sizes
above, to try the brush before the real builds are ready.

## Sizes of villages

The brush sizes the village by the selection: about 100 × 100 blocks gives
a hamlet of a handful of buildings round a well; 300 × 300, a village round
its square; 1000 × 1000, a town of several districts, each round its own
square, joined by main roads.
