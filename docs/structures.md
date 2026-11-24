# Structures

The **Structure** tool (Decor group) places the game's own structures with a
click: a village, an Ancient City with its warden, Trial Chambers, a
stronghold with its End portal, a shipwreck. Massif reads them from **your
own game** — the jar the launcher installed, and the jars of your mods — and
assembles them the way the game does. Nothing of Minecraft ships with Massif.

## Placing one

1. Pick **Structure** in the Decor group.
2. Choose what to place in Tool Settings: villages (plains, desert, savanna,
   taiga, snowy), a pillager outpost, the desert and jungle temples, a witch
   hut, an igloo, a shipwreck, ocean ruins (cold or warm), an Ancient City,
   Trial Chambers, a stronghold, trail ruins, a bastion remnant.
3. Point at the map: the outline of its pieces follows the pointer, with its
   size. Click to place it, on the map or in the 3D view.

Each placement is one step to undo. A right-click on a structure takes it
away (also one step). **Draw it again** puts another draw of the last one
in its place.

| Setting | What it does |
|---|---|
| **Structure** | What to place. |
| **Height** | For those placed at a depth (Ancient City, Trial Chambers, stronghold): the height of their start, the game's by default — an Ancient City's halls then lie between y −52 and y −22. |
| **Turn** | Any (drawn), or a quarter turn of your choice. |
| **Draw** | The draw it is assembled from: the same draw at the same place gives the same structure. The dice button draws another. |
| **Flatten the land under its buildings** | Villages, outposts, temples, igloos: the land under each building is raised or cut to its floor, with a slope of a few blocks to the land around. Paths follow the land as it then is. |

The **Placed** list under the settings names every structure on the map,
with where it is; its bin button takes one away.

## What comes with them

- **Chests with their loot tables**, filled by the game when first opened,
  as in a world it generated. Spawners, trial spawners and vaults keep their
  settings; suspicious sand and gravel keep their archaeologists' finds.
- **Villagers, animals, piglins**: the entities the game's templates hold
  (villages, outposts, bastions, the igloo's basement, a witch and her cat).
- **The Ancient City's warden.** Its patches of sculk grow shriekers the way
  the game grows them, with `can_summon=true`: only the world's own
  shriekers summon the warden, and these are. The city is carved out of the
  rock, the deep dark brought with it (its biome over the city's height).
- **Trial Chambers** are walled in by rock where a cave runs by, as the game
  does.

## The village brush

**Village brush** (Decor group) fills the selection with a village sized by
it: a hamlet round a well for about 100 × 100 blocks, a village round its
square for 300 × 300, a town of several districts for 1000 × 1000. Each
district has its square; main roads join them and lead out of the village,
finding their way over the land (gentle slopes and dry land first, planks
over water); side roads branch off them; the buildings stand along both,
their doors to the road, on ground levelled to their floor, with villagers
in the houses. **Walls** runs a wall round it, a few blocks inside the
selection's edge, with towers at its corners and along it, and a gate where
a road crosses.

Its themes are the game's own villages (plains, desert, savanna, taiga,
snowy: each district a village assembled by the jigsaw engine, kept to the
selection), or a builder's pack, added with **Add a pack…**: a folder of
schematics by theme and type with a small manifest, described in
[Village packs](village-packs.md).

A pack's village has more: farms with a field of crops behind them, a
market of stalls round the square, a mill by the water or toward the edge,
manors on high ground, and props — lampposts and benches along the roads,
hay and carts by the farms, fences round the fields. **Houses** leans the
village toward small or large buildings, **Props** sets how many props,
and **Roads** lays the roads in the theme's blocks, dirt, gravel, stone (with
slabs where they climb a block) or planks.

The village is built in six stages — the site (its squares on flat dry
land), the roads, the walls, the plots along the roads, the buildings, the
props — each its own step, which the terrain graph's Village Generator will
show as nodes. From the command line, `massif-cli village` takes the same
settings and more (`--houses`, `--size-bias`, `--road`, `--density`,
`--spread`, `--props`, `--fields`, `--mill`).

## Underground

An Ancient City, Trial Chambers or a stronghold carve their own halls: the
export writes their air over the rock. Caves (Edit ▸ Caves…) keep their
water and lava a block away from them, so that none runs in. They do not
show on the 2D map, which shows the land above them; their outline does,
while the Structure tool is in hand.

## Checking in the game

`tools/minecraft_check.py --structures` exports a map, starts the game's own
server on it and has the game confirm the structures' blocks, that their
chests keep their loot tables and that the game knows those tables, that the
Ancient City's shriekers can summon the warden, and that the villagers came
along (running the server means accepting Mojang's EULA).

```bash
tools/minecraft_check.py --world my.massif --version 1.21.11 --structures --accept-eula
```

## From the command line

```bash
massif-cli structure in.massif out.massif ancient_city 0 0 --seed 4
```

places one as the tool does (`--y` for its height, `--turn`, `--jar` for a
jar of your choice, `--keep-land` not to flatten). `massif-cli
structure-checks` lists what the game should find of them.

## For developers

The engine is the `massif-structures` crate:

- `library` reads `data/<namespace>/worldgen/template_pool`, `structure`
  (or `structures` before 1.21), `processor_list`, `configured_feature`,
  `placed_feature` and block tags from the game's jar and the mods' jars, a
  mod's file of the same path winning.
- `jigsaw` assembles as the game's `JigsawPlacement`: the start pool and
  its named jigsaw, depth (`size`), `max_distance_from_center`, jigsaws
  matched by name, target and joint (rollable or aligned), priorities, pool
  aliases, rooms of free space (each piece's own box for the pieces joined
  inside it), rigid pieces at their jigsaw's height and terrain-matching
  ones on the land, the villages' expansion hack, seeded draws.
- `processor` implements `rule` (with `append_loot`, `append_static`,
  `clear`; block, state, tag and random tests; linear position tests),
  `block_rot`, `block_ignore`, `gravity`, `protected_blocks`, `capped`,
  `jigsaw_replacement`, `block_age`, `blackstone_replace` and
  `lava_submerged_block`; any other is reported as unknown and leaves the
  blocks as they are.
- `build` lays the pieces (legacy pool elements leave the land where their
  template has air, as the game's do), carves `beard_box` pieces, walls in
  `encapsulate` ones, flattens under `beard_thin` ones; `features` grows
  trees, piles, patches and sculk patches from the game's feature files.
- `vanilla` is the catalogue of one-click structures; the igloo, shipwrecks
  and ocean ruins are assembled from their templates as the game's code
  does; `code` builds the temples, the witch hut and the stronghold with
  Massif's own code.

A placed structure is a `massif_core::structures::Structure` in
`Dimension::structures`: its blocks by 16³ section, block entities and
entities as NBT, its pieces' boxes. It has a section of its own in the
`.massif` file (`STRC`, `VSTR` in save points) and a pre-image in each step
of the history. The export stamps its blocks after the objects, writes its
block entities in each chunk, its entities in the entity files from 1.17
(in the chunk before), and the biome it brings as an override of the
chunk's 4³ biome cells.
