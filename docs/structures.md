# Structures

The **Structures** tool (Decor group) places villages and the game's own
structures: a village of the game's or of a builder's pack, an Ancient City
with its warden, Trial Chambers, a stronghold with its End portal, a
shipwreck. Massif reads the game's from **your own game** — the jar the
launcher installed, and the jars of your mods — and assembles them the way
the game does. Nothing of Minecraft ships with Massif.

## Placing one

1. Pick **Structures** in the Decor group.
2. In Tool Settings, pick what to place: under **Villages**, a theme (the
   game's plains, desert, savanna, taiga, snowy, or a pack's: **Add a
   pack…**); under **The game's structures**, a pillager outpost, the
   desert and jungle temples, a witch hut, an igloo, a shipwreck, ocean
   ruins, an Ancient City, Trial Chambers, a stronghold, trail ruins, a
   bastion remnant.
3. On the map:
   - **click** to place it there — a village of the size chosen, centred on
     the click;
   - or, for a village, **drag a rectangle**: the village fills it.
   With a selection on the map, **Fill the selection** builds a village in
   it, as the village brush did.

No selection is needed. Each placement is one step to undo.

## Changing one placed

Click a village or a structure placed: it is **selected** — its outline
in the accent colour, a village's four corners as handles — and its
settings show under **Selected** in Tool Settings.

- **Drag it** to move it: it is built again where you let go.
- **Drag a village's corner** to resize it: it is built again over the new
  rectangle at the same density, with more houses over more land and fewer
  over less.
- Change a village's **Size**, **Houses**, **Roads** (dirt, gravel, stone, or
  the pack's own), **Walls** or **Draw**: it is built again a moment after
  you stop changing them.
- Click another **theme** in the Villages list while a village is selected:
  it is built again of that theme, in its place.
- **Draw it again** (a structure of the game's) puts another draw in its
  place.
- **Take it away**, or a right-click on it, removes it.

Each is one step to undo. A village built again or taken away gives the
land back as it found it: the levelling under its buildings goes with it.
Villages and structures placed before 1.2 can be taken away but not moved
(they were not kept with what made them).

| Setting | What it does |
|---|---|
| **Size** | How close its houses stand: a small village's few and far apart, a large one's many and close. A click places a square of 112, 192 or 320 blocks (about 3, 12 and 48 houses); over any rectangle, the number of houses follows its area at that density. |
| **Houses** | How many houses the village holds, worked out from its area and size. Type a number to set it by hand: it then stays as typed when the village is resized, until the button beside it lets it follow the area again. |
| **Roads** | The blocks of its streets. |
| **Walls** | What runs round the village's buildings, a few blocks out from them, picked from a menu with a picture of each: none, a wooden fence, a spruce fence, post and rail, a hedge, a flowering hedge, a low stone wall, a dry-stone wall, stone and fence, a palisade, or the theme's ramparts with towers. Each is of the game's own blocks, with a little variety along its run (posts, lanterns, mossy stones, taller logs), and a gate where a road crosses it: a fence gate across the road, an arch of leaves over it, a beam over a palisade's. Villages placed before 1.2.1 keep their ramparts. |
| **Height** | For those placed at a depth (Ancient City, Trial Chambers, stronghold): the height of their start, the game's by default — an Ancient City's halls then lie between y −52 and y −22. |
| **Turn** | Any (drawn), or a quarter turn of your choice. |
| **Draw** | The draw it is assembled from: the same draw at the same place gives the same structure. The dice button draws another. |
| **Flatten the land under its buildings** | Outposts, temples, igloos: the land under each building is raised or cut to its floor. |

The **Placed** list names every structure on the map, with where it is; a
click on one selects it.

**Not yet:** the game's `/locate` and eyes of ender find only the
structures the game places itself — it looks for them where its world seed
puts them. Placing yours where the game would look is for a later version.
Woodland mansions and ocean monuments are not offered yet.

## Changing one building

The **Building** tool (Decor group, beside Structures) changes one building
of a village placed from a pack, the others left as they are:

- **Click** a building to select it: its outline in the accent colour, with
  handles at its corners and its name. Its picture, file and size show
  under **Selected** in Tool Settings.
- **Drag it** to move it. If its door no longer opens on a road, a path is
  laid from its door to the nearest road, around the other buildings.
- **R** (or **Turn**) turns it a quarter turn about its middle; a path leads
  to its door wherever the door then faces.
- **Delete** (or **Take it away**) removes it; a farm takes its field with
  it.
- **Another building** puts another building of the pack in its place, any
  type, picked from a menu of the pack's buildings with a picture of each
  seen from above, its size beside it. Its door stays on the same road.
- **Draw it again** puts another building of its type in its place.
- **As drawn** puts it back as the village's draw placed it.

Each change is one step to undo, and is kept with the village: built again
(moved, resized, of another theme or with other walls), the village makes
the same changes to the same buildings as long as its draw places them. A
change whose building is no longer in the draw (after changing the number
of houses, say) waits for it, and the tool says so. Only villages of a pack
placed with 1.2 or later can be changed building by building; the game's own
villages are assembled piece by piece by its jigsaw, and are changed whole.

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

## Villages

**Village brush** (Decor group) fills the selection with a village sized by
it: a hamlet round a well for about 100 × 100 blocks, a village round its
square for 300 × 300, a town of several districts for 1000 × 1000. Each
district has its square; main roads join them and lead out of the village,
finding their way over the land (gentle slopes and dry land first, planks
over water); side roads branch off them; the buildings stand along both,
their doors to the road, on ground levelled to their floor, with villagers
in the houses. **Walls** runs a fence, a hedge, a low wall, a palisade or
ramparts round it (the menu beside it), a few blocks inside the selection's
edge, with a gate where a road crosses.

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
`--spread`, `--props`, `--fields`, `--mill`, `--enclosure` with one of
`wooden-fence`, `spruce-fence`, `post-and-rail`, `hedge`, `flowering-hedge`,
`low-stone-wall`, `dry-stone-wall`, `stone-and-fence`, `palisade`,
`ramparts`). In the terrain graph, Village walls has the same **Style**, and
the Village Generator exposes it as **Wall style**.

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
