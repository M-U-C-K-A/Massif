# Importing a Minecraft world

**File ▸ Import Minecraft World…** (or **Import a Minecraft world…** on the home screen) turns a Minecraft Java world into a Massif map, as WorldPainter's *Import ▸ Existing Minecraft map* does. Pick the world's folder (the one holding `level.dat` and `region/`). Dropping the folder, its `level.dat` or one of its region files (`r.X.Z.mca`) on the window works too. The world is only read: save your work as a `.massif`.

Every chunk format Massif exports is read: 1.18 to 26.x (including the `dimensions/` layout of 26.1 and the `id`/`properties` keys of 26.3), 1.13 to 1.17, and the numbered blocks of 1.12 and before. Chunks compressed with gzip, zlib, LZ4 or not at all, and chunks too large for their region file (`c.X.Z.mcc`), are read.

## The dialog

- **Dimension**: the Overworld, the Nether or the End, those the world has.
- **Area**: **The whole world**, or **Drawn on the map**: the map shows every chunk the world's region files hold; drag over it to draw the area, or click to pick one region (512 × 512 blocks). The line under it says how many chunks and which blocks.
- **Heights**: the lowest and highest block kept (the dimension's own heights at first).
- **Above the ground**: what becomes of what stands on the land.
  - **Keep it**: buildings, trees, overhangs and everything else above the ground are kept block for block, with their states, as sculpted chunks (as 3D Sculpt makes): exported again, they come back unchanged. Each block gets a terrain of its own in the **Imported blocks** group.
  - **Trees as layers**: each kind of tree (oak, spruce, a mod's trees…) becomes an object layer made of up to twelve of the world's own trees, cut out of it, painted where that kind grew at the density it grew; buildings are left out.
  - **Drop it**: only the land comes in.
- **Plants, snow and carpets as ground covers**: a plant, a stack of sugar cane, seagrass or kelp, snow, a carpet or a mod's layer block growing on the ground becomes a ground cover of that block at full density (snow one layer deep is Frost), so it comes back where it was. Covers of layer blocks can then be made the ground's own in the import report, as with WorldPainter worlds.
- **Ground of mixed blocks as terrains that mix them**: where a chunk's ground mixes several blocks (stone and andesite, Conquest's dithered grounds), they become one terrain mixing them by their shares (or the map's terrain that already mixes them) rather than one terrain per block. Exported again, the mix is drawn anew, so the blocks are not where they were.
- **Biomes**: read per column, at the surface (the 4×4×4 biome cells of 1.18 and later, the columns of older worlds). A mod's or a data pack's biome keeps its id.
- **Caves and the underground, as sculpted chunks**: the caves, aquifers, ores and anything else under the ground are kept too. It takes much memory on a large world.

## What each column becomes

- **Height**: the top of its ground, the highest block that is land (stone, dirt, sand, a mod's ground…), under what grows or stands on it. In the Nether, the roof is skipped.
- **Terrain**: the block on top of the ground. A block the map's terrains already have on top (grass, sand, stone…) takes that terrain; any other gets a terrain of its own in the **Imported** group, with the block found under it as its second layer. A mod's blocks keep their ids.
- **Water and lava**: the water resting on the ground, to its level; frozen water is water under Frost; lava is water under the Lava cover.

## The import report

When the import is done, the report says what came in (the chunks, the heights, how long it took), the terrains it made, the ground covers and object layers, what it kept as sculpted chunks, and the blocks it left out with their counts. Chunks the game had not finished generating (at the world's edge) are skipped and counted.

## Large worlds

Regions are read side by side on every core, each turned into its tiles the moment it is read and packed at once, so memory stays flat with **Drop it** or **Trees as layers**; with **Keep it**, the sculpted chunks of what stands above the ground stay in memory. The import shows its progress and can be cancelled.

A map holds at most 80,000 sculpted chunks (about 80 million blocks of buildings and trees): over a larger world, **Keep it** keeps what stands on the first chunks read and leaves the rest out, as the report says. Draw a smaller area to keep it all, or take the trees as layers; the dialog picks **Trees as layers** at first for a world of more than 60,000 chunks.

Measured on a Mac with an M-series chip, the world on an external SSD:

| World | Chunks | Above the ground | Time | Memory at most |
|---|---|---|---|---|
| 1.21.11, survival | 26,831 | Drop it | 2.4 s | 218 MB |
| 1.21.11, survival | 26,831 | Trees as layers | 2.7 s | 305 MB |
| 1.21.11, survival, with a town | 26,831 | Keep it (3.6 million blocks) | 2.6 s | 658 MB |
| 1.21.11, about 8,200 × 8,200 blocks | 264,081 | Drop it | 28.8 s | 331 MB |
| 1.21.11, about 8,200 × 8,200 blocks | 264,081 | Keep it (16.8 million blocks, capped) | 32.8 s | 2.1 GB |

## Exported again

A map imported and exported again gives back the same heights, top blocks, water and biomes, and with **Keep it** the same blocks above the ground, state for state, for every version from 1.13 on. Before 1.13 the export writes blocks by their numbers without their states (the way stairs face, the upper half of tall plants), as for any map. The game itself confirms the top blocks, water and biomes of the export (`tools/minecraft_check.py`); a biome can differ at the edge of a 4 × 4 cell, as the export writes one biome a cell.
